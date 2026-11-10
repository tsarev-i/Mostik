//! Context menu and developer tools of the embedded browser.
//!
//! As in Slack, the standard browser menu ("Back", "Reload", "Open Link in New Window",
//! "Inspect Element") is never shown. The UI draws the message and sidebar menus itself,
//! and of the system menu only the editing items survive: in text fields, on selected text,
//! on links and images. Where there are no such items, no menu appears at all.
//!
//! Developer tools are enabled only in debug builds with `MOSTIK_DEVTOOLS=1`
//! (Slack's `SLACK_DEVELOPER_MENU` works the same way): then "Inspect Element" returns to the menu.

use tauri::{Runtime, WebviewWindow};

/// Environment variable that enables developer tools in a debug build.
pub const DEVTOOLS_ENV: &str = "MOSTIK_DEVTOOLS";

/// Whether developer tools are allowed in this run.
pub fn devtools_enabled() -> bool {
    cfg!(debug_assertions) && std::env::var(DEVTOOLS_ENV).is_ok_and(|v| v == "1")
}

/// Sets up the menu and developer tools of a window. Called for every created window.
pub fn harden<R: Runtime>(window: &WebviewWindow<R>) {
    let devtools = devtools_enabled();
    if let Err(e) = window.with_webview(move |pw| imp::harden(&pw, devtools)) {
        eprintln!("could not set up the window context menu: {e}");
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use tauri::webview::PlatformWebview;
    use webkit2gtk::{ContextMenuAction as A, ContextMenuExt, ContextMenuItemExt, SettingsExt, WebViewExt};

    /// System menu items that Slack also has: text editing, spelling, copying links and images.
    fn allowed(action: A, devtools: bool) -> bool {
        match action {
            A::Copy
            | A::Cut
            | A::Paste
            | A::PasteAsPlainText
            | A::Delete
            | A::SelectAll
            | A::SpellingGuess
            | A::NoGuessesFound
            | A::IgnoreSpelling
            | A::LearnSpelling
            | A::InsertEmoji
            | A::CopyLinkToClipboard
            | A::CopyImageToClipboard
            | A::CopyImageUrlToClipboard => true,
            A::InspectElement => devtools,
            _ => false,
        }
    }

    pub fn harden(pw: &PlatformWebview, devtools: bool) {
        let view = pw.inner();
        if let Some(settings) = WebViewExt::settings(&view) {
            settings.set_enable_developer_extras(devtools);
        }
        view.connect_context_menu(move |_, menu, _, _| {
            for item in menu.items() {
                if !item.is_separator() && !allowed(item.stock_action(), devtools) {
                    menu.remove(&item);
                }
            }
            // Separators at the edges and in a row stay dangling after the cleanup: remove them.
            let mut prev_sep = true;
            for item in menu.items() {
                if item.is_separator() {
                    if prev_sep {
                        menu.remove(&item);
                    }
                    prev_sep = true;
                } else {
                    prev_sep = false;
                }
            }
            if let Some(last) = menu.last().filter(|i| i.is_separator()) {
                menu.remove(&last);
            }
            // `true` means do not show the menu.
            menu.n_items() == 0
        });
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use tauri::webview::PlatformWebview;

    pub fn harden(_pw: &PlatformWebview, _devtools: bool) {}
}
