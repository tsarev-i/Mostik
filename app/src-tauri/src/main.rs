//! Mostik desktop shell: windows, tray, calls, the `mostikfile://` scheme and the IPC commands.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod appimage;
mod backend;
mod call;
#[cfg(target_os = "linux")]
mod cdp;
mod downloads;
mod login;
mod ring;
mod sound;
mod store;
mod update;
mod webview;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

/// Shows, unminimizes and focuses the main window.
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
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
    tray.build(app)?;
    Ok(())
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
            // Files dropped onto the window, as in Slack, are attached to the message.
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                let files = backend::allow_files(window.app_handle(), paths);
                if !files.is_empty() {
                    let _ = window.app_handle().emit(backend::FILES_EVENT, &files);
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
            backend::bootstrap,
            backend::login,
            backend::logout,
            backend::history,
            backend::mark_read,
            backend::send_message,
            backend::edit_message,
            backend::service::custom_emoji,
            backend::service::pick_emoji_image,
            backend::service::add_custom_emoji,
            backend::service::rename_custom_emoji,
            backend::service::delete_custom_emoji,
            backend::delete_message,
            backend::set_reaction,
            backend::set_important,
            backend::typing,
            backend::open_url,
            backend::search_messages,
            backend::set_notify_level,
            backend::set_starred,
            backend::save_chat_folders,
            backend::mark_unread,
            backend::pin_message,
            backend::forward_message,
            backend::start_call,
            backend::focus_call,
            backend::leave_call,
            backend::current_call,
            ring::ring_info,
            ring::answer_ring,
            backend::recommended_chats,
            backend::join_chat,
            backend::open_yandex_id,
            backend::save_file,
            backend::test_notification,
            backend::play_sound,
            backend::followed_threads,
            backend::mark_thread_read,
            backend::set_preferences,
            backend::set_zoom,
            backend::set_window_title,
            backend::user_profile,
            backend::open_dm,
            backend::search_people,
            backend::create_channel,
            backend::chat_members,
            backend::change_members,
            backend::update_chat,
            backend::leave_chat,
            backend::chat_details,
            backend::pick_files,
            backend::keep_file,
            backend::send_files,
            backend::presence,
            backend::set_status,
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
            }
        });
}
