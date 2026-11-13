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
        let Some(target) = performed
            .notification()
            .and_then(|n| backend::OpenTarget::from_extra(n.extra()))
        else {
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
            match drag {
                tauri::DragDropEvent::Enter { position, .. } | tauri::DragDropEvent::Over { position } => {
                    let _ = app.emit(backend::DRAG_EVENT, Some((position.x, position.y)));
                }
                tauri::DragDropEvent::Drop { paths, position } => {
                    let _ = app.emit(backend::DRAG_EVENT, None::<(f64, f64)>);
                    let files = backend::allow_files(app, paths);
                    if !files.is_empty() {
                        let dropped = backend::Dropped {
                            files,
                            x: position.x,
                            y: position.y,
                        };
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
            backend::service::rename_custom_emoji,
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
