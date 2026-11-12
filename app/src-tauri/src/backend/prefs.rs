//! Settings from the UI: the window title and zoom, and the preferences the backend needs.

use tauri::{AppHandle, Manager};

use super::{AppState, NotifyMode, Prefs, TrayItems, text};

/// Main window title: the UI language and the unread count, as in Slack.
#[tauri::command]
pub fn set_window_title(app: AppHandle, title: String) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window.set_title(&title).map_err(|e| e.to_string())
}

/// UI zoom from "Preferences → Accessibility" and Ctrl+= / Ctrl+-.
#[tauri::command]
pub fn set_zoom(app: AppHandle, scale: f64) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window.set_zoom(scale.clamp(0.5, 2.0)).map_err(|e| e.to_string())
}

/// Settings from the UI that the backend needs: notifications, previews and language.
#[tauri::command]
pub fn set_preferences(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    notify: NotifyMode,
    preview: bool,
    lang: String,
    sound: Option<String>,
) {
    let english = lang == "en";
    if let Ok(mut p) = state.prefs.lock() {
        *p = Prefs {
            notify,
            preview,
            english,
            sound: sound.unwrap_or_else(|| p.sound.clone()),
        };
    }
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.open.set_text(text(&app, "Открыть Мостик", "Open Mostik"));
        let _ = items.quit.set_text(text(&app, "Выйти", "Quit"));
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(text(
            &app,
            "Мостик | неофициальный клиент Телемост",
            "Mostik | unofficial Telemost client",
        )));
    }
}
