//! Mostik desktop shell: windows, tray, calls, the `mostikfile://` scheme and the IPC commands.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod appimage;
mod backend;
mod badge;
mod call;
#[cfg(target_os = "linux")]
mod cdp;
mod downloads;
mod login;
mod ring;
mod sound;
mod update;
mod webview;

use mostik_app::store;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

/// Shows, unminimizes and focuses the main window.
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        webview::raise(&w);
    }
}

/// A click on a message notification shows the main window on that message. On Wayland the
/// window cannot take the focus by itself: it asks for attention instead.
fn open_from_notification(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.notification().on_action(move |performed| {
        if performed.action_id() != "tap" {
            return;
        }
        let Some(target) = performed.notification().and_then(|n| backend::open_target(n.extra())) else {
            return;
        };
        let app = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            show_main(&app);
            if let Some(w) = app.get_webview_window("main")
                && !w.is_focused().unwrap_or(false)
            {
                let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
            }
            let _ = app.emit(backend::OPEN_EVENT, &target);
        });
    });
}

/// Tray icon with "Open" and "Quit"; a left click shows the main window.
fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(
        app,
        "open",
        backend::text(app, "Открыть Мостик", "Open Mostik"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", backend::text(app, "Выйти", "Quit"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    app.manage(backend::TrayItems {
        open: open.clone(),
        quit: quit.clone(),
    });
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip(backend::text(
            app,
            "Мостик | неофициальный клиент Телемост",
            "Mostik | unofficial Telemost client",
        ))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    #[cfg(target_os = "linux")]
    {
        clear_tray_dirs();
        tray = tray.temp_dir_path(tray_dir());
    }
    tray.build(app)?;
    Ok(())
}

/// Where the tray icon is painted on Linux: the panel reads it from a file. Each running copy has a
/// folder of its own: in the shared `$XDG_RUNTIME_DIR/tray-icon` two copies overwrite and delete
/// each other's `tray-icon-main-N.png`, so the unread mark of one shows on the other or vanishes.
#[cfg(target_os = "linux")]
fn tray_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("mostik-tray-{}", std::process::id()))
}

/// Removes the tray folders of copies that are gone: one stopped by a signal leaves its folder.
#[cfg(target_os = "linux")]
fn clear_tray_dirs() {
    let dir = tray_dir();
    let Some(Ok(entries)) = dir.parent().map(std::fs::read_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.strip_prefix("mostik-tray-")) else {
            continue;
        };
        if pid.parse::<u32>().is_ok() && !std::path::Path::new("/proc").join(pid).exists() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The main window, as `tauri.conf.json` describes it (`"create": false` there), kept on the
/// app's own pages: links from messages open in the default browser, never in its place.
fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main") else {
        return Ok(());
    };
    let window = WebviewWindowBuilder::from_config(app, config)?
        .on_navigation(|url| {
            let own = webview::is_app_url(url);
            if !own {
                eprintln!(
                    "main window: refused to leave the app for {}",
                    url.origin().ascii_serialization()
                );
            }
            own
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build()?;
    webview::harden(&window);
    webview::forget_site_data(&window, webview::SiteData::Cookies);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(backend::AppState::default())
        .manage(call::CallState::default())
        .manage(ring::RingState::default())
        .manage(update::UpdateState::default())
        .register_asynchronous_uri_scheme_protocol("mostikfile", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_owned();
            let query = request.uri().query().map(str::to_owned);
            tauri::async_runtime::spawn(async move {
                let response = match backend::serve_file(&app, &path, query.as_deref()).await {
                    Ok((bytes, kind)) => tauri::http::Response::builder()
                        .header("Content-Type", kind)
                        .header("Cache-Control", "max-age=3600")
                        .body(bytes),
                    Err(e) => tauri::http::Response::builder().status(404).body(e.into_bytes()),
                };
                if let Ok(r) = response {
                    responder.respond(r);
                }
            });
        })
        .on_window_event(|window, event| {
            // Files dropped onto the chat, as in Slack, are attached to the message. The page
            // learns where the drag is, to highlight the chat and to decide where the files go.
            // The other windows (the call and sign-in windows) take no files.
            let tauri::WindowEvent::DragDrop(drag) = event else {
                return;
            };
            if window.label() != "main" {
                return;
            }
            let app = window.app_handle();
            // The page gets the point in logical pixels of the window. Tauri calls it physical,
            // but passes on what the webview reports: physical pixels only on Windows; GTK and
            // AppKit give logical ones, and scaling them again missed the pane under the cursor on
            // a scaled screen.
            let scale = if cfg!(windows) {
                window.scale_factor().unwrap_or(1.0)
            } else {
                1.0
            };
            let logical = |p: &tauri::PhysicalPosition<f64>| (p.x / scale, p.y / scale);
            match drag {
                tauri::DragDropEvent::Enter { position, .. } | tauri::DragDropEvent::Over { position } => {
                    let _ = app.emit(backend::DRAG_EVENT, Some(logical(position)));
                }
                tauri::DragDropEvent::Drop { paths, position } => {
                    let _ = app.emit(backend::DRAG_EVENT, None::<(f64, f64)>);
                    let files = backend::allow_files(app, paths);
                    if !files.is_empty() {
                        let (x, y) = logical(position);
                        let dropped = backend::Dropped { files, x, y };
                        let _ = app.emit(backend::FILES_EVENT, &dropped);
                    }
                }
                _ => {
                    let _ = app.emit(backend::DRAG_EVENT, None::<(f64, f64)>);
                }
            }
        })
        .setup(|app| {
            create_main_window(app.handle())?;
            // Before anything can notify: clicks are reported only for notifications shown
            // after a handler exists.
            open_from_notification(app.handle());
            // The tray is optional: without an app indicator in the desktop environment we simply run without it.
            if let Err(e) = setup_tray(app.handle()) {
                eprintln!("tray icon unavailable: {e}");
            }
            call::cleanup(app.handle());
            update::start(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend::connection::bootstrap,
            backend::connection::login,
            backend::connection::logout,
            backend::messages::history,
            backend::messages::mark_read,
            backend::messages::send_message,
            backend::messages::edit_message,
            backend::service::custom_emoji,
            backend::service::pick_emoji_image,
            backend::service::add_custom_emoji,
            backend::service::delete_custom_emoji,
            backend::messages::delete_message,
            backend::messages::set_reaction,
            backend::messages::set_important,
            backend::messages::typing,
            backend::files::open_url,
            backend::chats::search_messages,
            backend::chats::set_notify_level,
            backend::chats::set_starred,
            backend::chats::save_chat_folders,
            backend::messages::mark_unread,
            backend::messages::pin_message,
            backend::messages::forward_message,
            backend::calls::start_call,
            backend::calls::focus_call,
            backend::calls::leave_call,
            backend::calls::current_call,
            ring::ring_info,
            ring::answer_ring,
            backend::chats::recommended_chats,
            backend::chats::join_chat,
            backend::files::open_yandex_id,
            backend::files::save_file,
            backend::notify::test_notification,
            backend::notify::play_sound,
            backend::threads::followed_threads,
            backend::threads::mark_thread_read,
            backend::prefs::set_preferences,
            backend::prefs::set_zoom,
            backend::prefs::set_window_title,
            backend::prefs::set_unread_badge,
            backend::chats::user_profile,
            backend::chats::open_dm,
            backend::chats::search_people,
            backend::chats::create_channel,
            backend::chats::chat_members,
            backend::chats::change_members,
            backend::chats::update_chat,
            backend::chats::leave_chat,
            backend::chats::chat_details,
            backend::files::pick_files,
            backend::files::keep_file,
            backend::files::forget_file,
            backend::files::send_files,
            backend::presence::presence,
            backend::presence::set_status,
            update::update_status,
            update::install_update,
            update::set_auto_update,
            update::check_update,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start Mostik")
        .run(|app, event| {
            // The call browser and its profile with the session cookies go with the app.
            if let tauri::RunEvent::Exit = event {
                call::shutdown(app);
                #[cfg(target_os = "linux")]
                let _ = std::fs::remove_dir_all(tray_dir());
            }
        });
}

#[cfg(test)]
mod tests {
    use mostik_ipc::cmd;

    /// Tauri knows a command by the name of its function, which the compiler cannot match with
    /// the `Command` types the UI calls: the handler must register exactly [`cmd::ALL`].
    #[test]
    fn registered_commands_are_the_shared_ones() {
        let src = include_str!("main.rs");
        let start = src.find("generate_handler![").expect("handler list") + "generate_handler![".len();
        let end = start + src[start..].find(']').expect("end of the handler list");
        // A commented-out line is not registered.
        let list: String = src[start..end]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut registered: Vec<&str> = list
            .split(',')
            .filter_map(|path| path.trim().rsplit("::").next())
            .filter(|name| !name.is_empty())
            .collect();
        registered.sort_unstable();
        let mut shared: Vec<&str> = cmd::ALL.iter().map(|c| c.name).collect();
        shared.sort_unstable();
        assert_eq!(registered, shared);
    }

    /// The parameters a command may take: the app, its state, the raw request (`keep_file`) and
    /// its arguments as one struct, [`cmd::ARGS`]. Anything else would be a top-level JSON key
    /// the UI does not send.
    const PARAMS: [&str; 4] = ["app", "state", "request", cmd::ARGS];

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("source folder") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Splits `params` at the commas outside `<…>` and `(…)`.
    fn split_params(params: &str) -> Vec<&str> {
        let (mut depth, mut start, mut out) = (0i32, 0, Vec::new());
        for (i, c) in params.char_indices() {
            match c {
                '<' | '(' => depth += 1,
                '>' | ')' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(params[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
        out.push(params[start..].trim());
        out.into_iter().filter(|p| !p.is_empty()).collect()
    }

    /// The compiler ties a command function to its `Command` type through `Args<cmd::X>` and
    /// `Out<cmd::X>`; what it cannot check is checked here: the function is named as the type's
    /// `NAME`, takes `Args` and returns `Out` of that same type (not another command's), takes
    /// `args` exactly when the command has JSON arguments, and has no other parameters than
    /// [`PARAMS`].
    #[test]
    fn commands_take_their_shared_args() {
        // Built from parts, so this test's own text is not taken for a command.
        let marker = concat!("#[tauri", "::command]");
        let mut files = Vec::new();
        rust_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut seen = Vec::new();
        for file in files {
            let src = std::fs::read_to_string(&file).expect("source file");
            for (at, _) in src.match_indices(marker) {
                // Only the attribute on a line of its own, not a mention in a comment.
                let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
                if !src[line_start..at].trim().is_empty() {
                    continue;
                }
                let sig = &src[at + marker.len()..];
                let sig = &sig[..sig.find('{').expect("function body")];
                let name_at = sig.find("fn ").expect("fn") + 3;
                let open = name_at + sig[name_at..].find('(').expect("parameters");
                let name = sig[name_at..open].trim();
                let mut depth = 0;
                let close = open
                    + sig[open..]
                        .char_indices()
                        .find(|&(_, c)| {
                            depth += match c {
                                '(' => 1,
                                ')' => -1,
                                _ => 0,
                            };
                            depth == 0
                        })
                        .expect("end of the parameters")
                        .0;
                let (mut args, mut request) = (None, false);
                for param in split_params(&sig[open + 1..close]) {
                    let (param, ty) = param.split_once(':').expect("typed parameter");
                    let param = param.trim().trim_start_matches("mut ");
                    assert!(PARAMS.contains(&param), "{name}: parameter `{param}`");
                    if param == cmd::ARGS {
                        args = Some(ty.trim().to_owned());
                    }
                    request |= param == "request";
                }
                let info = cmd::ALL
                    .iter()
                    .find(|c| c.name == name)
                    .unwrap_or_else(|| panic!("{name}: not in cmd::ALL"));
                let bare = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
                let ty = info.ty;
                // `()` takes nothing; `RawBody` (`keep_file`) reads the raw request instead.
                let expected_args = (!matches!(info.args, "()" | "RawBody")).then(|| format!("Args<cmd::{ty}>"));
                assert_eq!(args.map(|a| bare(&a)), expected_args, "{name}: arguments");
                // The raw request is for a command whose body is not JSON, and such a command needs it.
                assert_eq!(request, info.args == "RawBody", "{name}: `request` parameter");
                let ret = bare(sig[close + 1..].trim().strip_prefix("->").unwrap_or(""));
                assert!(
                    ret == format!("Result<Out<cmd::{ty}>,String>") || ret == format!("Out<cmd::{ty}>"),
                    "{name}: returns `{ret}`, not `Out<cmd::{ty}>`"
                );
                seen.push(name.to_owned());
            }
        }
        seen.sort_unstable();
        let mut all: Vec<String> = cmd::ALL.iter().map(|c| c.name.to_owned()).collect();
        all.sort_unstable();
        assert_eq!(seen, all);
    }
}
