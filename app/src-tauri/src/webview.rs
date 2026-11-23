//! Context menu, navigation, developer tools and site data of the embedded browser.
//!
//! As in Slack, the standard browser menu ("Back", "Reload", "Open Link in New Window",
//! "Inspect Element") is never shown. The UI draws the message and sidebar menus itself,
//! and of the system menu only the editing items survive: in text fields, on selected text,
//! on links and images. Where there are no such items, no menu appears at all. That is so on
//! Linux and Windows; on macOS the system menu stays as it is, and [`is_app_url`] keeps its
//! "Open Link" from taking over the window (new windows and downloads are refused there anyway).
//!
//! Developer tools exist only in debug builds; with `MOSTIK_DEVTOOLS=1` (Slack's
//! `SLACK_DEVELOPER_MENU` works the same way) "Inspect Element" returns to the menu on Linux and
//! Windows.

use tauri::{Runtime, Url, WebviewWindow};

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

/// Brings a window to the front: shown, unminimized and focused ("Open Mostik" in the tray, a
/// click on a notification, the Dock icon).
///
/// tao asks for the focus only for a window it knows as shown and not minimized, and the unminimize
/// sent just before has not changed that yet, so the focus is asked for once more a moment later.
/// On Linux that is not enough under Wayland: GTK cannot unminimize a window there, and the
/// compositor brings a window forward only with an activation token from the user's click, which
/// the tray (libayatana) never gets; "Open Mostik" did nothing. Under KDE Plasma KWin is then
/// asked to activate the window by a script of its own (see `imp::kwin_activate`), which its focus
/// stealing prevention does not stop; elsewhere the window asks for attention (its taskbar button
/// lights up).
pub fn raise<R: Runtime>(window: &WebviewWindow<R>) {
    let in_front = window.is_visible().unwrap_or(false)
        && window.is_focused().unwrap_or(false)
        && !window.is_minimized().unwrap_or(false);
    if in_front {
        return;
    }
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        // Once in front, it is done: what the user does next (another window, minimizing) is not
        // undone by a retry.
        if got_focus(&window).await {
            return;
        }
        #[cfg(target_os = "linux")]
        {
            let title = window.title().unwrap_or_default();
            match tauri::async_runtime::spawn_blocking(move || imp::kwin_activate(&title)).await {
                Ok(Ok(())) => {
                    if got_focus(&window).await {
                        return;
                    }
                }
                Ok(Err(e)) => eprintln!("window not raised by KWin: {e}"),
                Err(e) => eprintln!("window not raised by KWin: {e}"),
            }
            let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
        }
        #[cfg(not(target_os = "linux"))]
        let _ = window.set_focus();
    });
}

/// Whether the window gets the focus within a moment, checked every 50 ms.
async fn got_focus<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    for _ in 0..8 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if window.is_focused().unwrap_or(true) {
            return true;
        }
    }
    false
}

/// Whether `url` is a page of the app itself: `tauri://localhost` on Linux and macOS,
/// `http://tauri.localhost` on Windows. The main window shows nothing else. Links from messages
/// open in the default browser; a navigation that slips past the UI (a middle click, a link
/// while the UI is down, "Open Link" of a system menu) would put someone else's page in place
/// of the app, in the app's own window.
pub fn is_app_url(url: &Url) -> bool {
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("tauri.localhost"),
        _ => false,
    }
}

/// What [`forget_site_data`] clears.
#[derive(Clone, Copy)]
pub enum SiteData {
    Cookies,
    /// Cookies, caches and site databases: everything but `localStorage`.
    CookiesAndCaches,
}

/// Clears site data of a window's browser profile. `localStorage` stays: the UI keeps its
/// preferences there and clears the account's own keys itself. The main window needs no cookies
/// (the backend makes every request that needs the session), but earlier versions left Yandex ID
/// cookies in its profile: the login window and, on Windows, the call window shared it.
pub fn forget_site_data<R: Runtime>(window: &WebviewWindow<R>, what: SiteData) {
    if let Err(e) = window.with_webview(move |pw| imp::forget_site_data(&pw, what)) {
        eprintln!("could not clear the site data: {e}");
    }
}

/// Turns spell checking of the window's text fields on or off (#50). Only WebKitGTK needs it: it
/// checks nothing until its web context is given languages. They are Russian and English, the
/// languages people write in Telemost, and the system's. The Russian and English dictionaries come
/// with Mostik (see `spelling.rs`), the others from the system (enchant, hunspell); a language
/// without one is skipped. Elsewhere the fields' `spellcheck`
/// attribute, which the UI sets from the same preference, is enough.
pub fn spell_check<R: Runtime>(window: &WebviewWindow<R>, enabled: bool) {
    #[cfg(target_os = "linux")]
    {
        let languages = spelling_languages(&system_locale());
        if let Err(e) = window.with_webview(move |pw| imp::spell_check(&pw, enabled, &languages)) {
            eprintln!("could not set up spell checking: {e}");
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, enabled);
}

/// The locale of the user's messages: `LC_ALL`, `LC_MESSAGES` or `LANG`, as gettext reads them.
#[cfg(target_os = "linux")]
fn system_locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// Enchant language codes: Russian, English and the system's (`de_DE.UTF-8` gives `de_DE`), each once.
#[cfg(any(target_os = "linux", test))]
fn spelling_languages(locale: &str) -> Vec<String> {
    let system = locale.split(['.', '@']).next().unwrap_or_default();
    let mut languages = vec!["ru_RU".to_owned(), "en_US".to_owned()];
    let plain = |l: &str| l.len() >= 2 && l.chars().all(|c| c.is_ascii_alphabetic() || c == '_');
    if plain(system) && !matches!(system, "C" | "POSIX") && !languages.iter().any(|l| l == system) {
        languages.push(system.to_owned());
    }
    languages
}

/// The KWin script that unminimizes and activates the window of process `pid` titled `title`
/// (KWin may add ` <2>` to a title another window has too). KWin 5 names things otherwise.
#[cfg(any(target_os = "linux", test))]
fn kwin_script(pid: u32, title: &str) -> String {
    let title = serde_json::to_string(title).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        r#"const title = {title};
const windows = workspace.windowList ? workspace.windowList() : workspace.clientList();
for (const w of windows) {{
    if (w.pid !== {pid} || !(w.caption === title || w.caption.startsWith(title + " <"))) continue;
    w.minimized = false;
    if ("activeWindow" in workspace) workspace.activeWindow = w; else workspace.activeClient = w;
}}
"#
    )
}

/// Separators to remove from a menu whose items are separators (`true`) or not: those at either
/// end and all but the first of a run. Indices from the last, so removing them in that order
/// keeps the rest in place.
#[cfg(any(windows, test))]
fn dangling_separators(separators: &[bool]) -> Vec<usize> {
    let last_item = separators.iter().rposition(|&sep| !sep);
    let mut prev_sep = true;
    let mut remove: Vec<usize> = (0..separators.len())
        .filter(|&i| {
            let sep = separators[i];
            let dangling = sep && (prev_sep || last_item.is_none_or(|last| i > last));
            prev_sep = sep;
            dangling
        })
        .collect();
    remove.reverse();
    remove
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

    pub fn spell_check(pw: &PlatformWebview, enabled: bool, languages: &[String]) {
        use webkit2gtk::WebContextExt;

        let Some(context) = WebViewExt::context(&pw.inner()) else {
            return;
        };
        if enabled {
            let languages: Vec<&str> = languages.iter().map(String::as_str).collect();
            context.set_spell_checking_languages(&languages);
        }
        context.set_spell_checking_enabled(enabled);
    }

    /// Asks KWin to unminimize and activate this process's window titled `title`, by loading a
    /// script into it over D-Bus (`org.kde.kwin.Scripting`), as `kdotool` does: a script runs
    /// inside KWin, so focus stealing prevention, which refuses a window that asks by itself
    /// without the user's activation token, does not apply. Only under KDE Plasma; an error
    /// elsewhere or when KWin does not answer.
    pub fn kwin_activate(title: &str) -> Result<(), String> {
        use webkit2gtk::gio::{self, BusType, DBusCallFlags};
        use webkit2gtk::glib::{ToVariant, Variant, VariantTy};

        let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|d| d == "KDE"));
        if !kde {
            return Err("not KDE Plasma".into());
        }
        // One at a time: clicks in a row would share the script's name and file, and one would
        // unload or delete the other's.
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let pid = std::process::id();
        // The user's own folder only: in a shared one, such as `/tmp`, another user could put a
        // script of theirs at the name. Plasma always has one.
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .filter(|d| d.is_absolute())
            .ok_or("no XDG_RUNTIME_DIR")?;
        let path = dir.join(format!("mostik-raise-{pid}.js"));
        // A file of an earlier raise that went wrong; then a new one, open to the user only.
        let _ = std::fs::remove_file(&path);
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|e| e.to_string())?;
            file.write_all(super::kwin_script(pid, title).as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let path_text = path.to_string_lossy().into_owned();
        let name = format!("mostik-raise-{pid}");
        let bus = gio::bus_get_sync(BusType::Session, None::<&gio::Cancellable>).map_err(|e| e.to_string())?;
        let call = |object: &str, method: &str, args: Option<&Variant>, reply: Option<&VariantTy>| {
            let interface = if object == "/Scripting" {
                "org.kde.kwin.Scripting"
            } else {
                "org.kde.kwin.Script"
            };
            bus.call_sync(
                Some("org.kde.KWin"),
                object,
                interface,
                method,
                args,
                reply,
                DBusCallFlags::NO_AUTO_START,
                2000,
                None::<&gio::Cancellable>,
            )
            .map_err(|e| e.to_string())
        };
        let unload = || call("/Scripting", "unloadScript", Some(&(name.as_str(),).to_variant()), None);
        // One left by an earlier raise that failed halfway.
        let _ = unload();
        let reply = VariantTy::new("(i)").map_err(|e| e.to_string())?;
        let loaded = call(
            "/Scripting",
            "loadScript",
            Some(&(path_text.as_str(), name.as_str()).to_variant()),
            Some(reply),
        );
        // KWin reads the file when the script runs, and in the background: the file and the
        // script stay a moment longer.
        let ran = loaded.and_then(|loaded| {
            let (id,): (i32,) = loaded.get().ok_or("KWin gave no script id")?;
            if id < 0 {
                return Err("KWin did not load the script".to_owned());
            }
            // KWin 6 puts a script at `/Scripting/Script<id>`, KWin 5 at `/<id>`.
            call(&format!("/Scripting/Script{id}"), "run", None, None)
                .or_else(|_| call(&format!("/{id}"), "run", None, None))
        });
        std::thread::sleep(std::time::Duration::from_millis(500));
        let _ = unload();
        let _ = std::fs::remove_file(&path);
        ran.map(drop)
    }

    pub fn forget_site_data(pw: &PlatformWebview, what: super::SiteData) {
        use webkit2gtk::{WebsiteDataManagerExtManual, WebsiteDataTypes as T, gio, glib};

        let Some(manager) = WebViewExt::website_data_manager(&pw.inner()) else {
            return;
        };
        let types = match what {
            super::SiteData::Cookies => T::COOKIES,
            super::SiteData::CookiesAndCaches => {
                T::COOKIES
                    | T::DISK_CACHE
                    | T::MEMORY_CACHE
                    | T::OFFLINE_APPLICATION_CACHE
                    | T::INDEXEDDB_DATABASES
                    | T::WEBSQL_DATABASES
                    | T::SERVICE_WORKER_REGISTRATIONS
                    | T::DOM_CACHE
                    | T::HSTS_CACHE
            }
        };
        // A time span of zero means data of any age.
        manager.clear(types, glib::TimeSpan(0), None::<&gio::Cancellable>, |result| {
            if let Err(e) = result {
                eprintln!("could not clear the site data: {e}");
            }
        });
    }
}

#[cfg(windows)]
mod imp {
    use tauri::webview::PlatformWebview;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::{ClearBrowsingDataCompletedHandler, ContextMenuRequestedEventHandler, take_pwstr};
    use windows_core::{Interface, PWSTR};

    /// Menu items Slack also has, by their WebView2 names (the English label in lower camel
    /// case): text editing, spelling, emoji, copying links and images.
    const ALLOWED: [&str; 14] = [
        "undo",
        "redo",
        "cut",
        "copy",
        "paste",
        "pasteAndMatchStyle",
        "selectAll",
        "emoji",
        "spellCheck",
        "copyLink",
        "copyLinkLocation",
        "copyImage",
        "copyImageLink",
        "copyImageLocation",
    ];

    fn allowed(name: &str, devtools: bool) -> bool {
        ALLOWED.contains(&name) || (devtools && matches!(name, "inspect" | "inspectElement"))
    }

    pub fn harden(pw: &PlatformWebview, devtools: bool) {
        // SAFETY: WebView2 COM calls on the webview's own thread, where `with_webview` runs; the
        // handler runs on that thread too.
        let result = unsafe {
            (|| -> windows_core::Result<()> {
                let webview: ICoreWebView2_11 = pw.controller().CoreWebView2()?.cast()?;
                let mut token = 0;
                webview.add_ContextMenuRequested(
                    &ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
                        let Some(args) = args else { return Ok(()) };
                        let items = args.MenuItems()?;
                        prune(&items, devtools)?;
                        let mut left = 0;
                        items.Count(&mut left)?;
                        // Nothing left: no menu at all.
                        if left == 0 {
                            args.SetHandled(true)?;
                        }
                        Ok(())
                    })),
                    &mut token,
                )
            })()
        };
        if let Err(e) = result {
            eprintln!("could not set up the window context menu: {e}");
        }
    }

    /// Whether each item of the menu is a separator.
    fn separators(items: &ICoreWebView2ContextMenuItemCollection) -> windows_core::Result<Vec<bool>> {
        let mut count = 0;
        // SAFETY: COM calls on the webview's thread (the context menu handler).
        unsafe { items.Count(&mut count)? };
        (0..count)
            .map(|i| {
                let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
                // SAFETY: as above.
                unsafe { items.GetValueAtIndex(i)?.Kind(&mut kind)? };
                Ok(kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR)
            })
            .collect()
    }

    /// Removes the items not [`allowed`], then the separators left dangling.
    fn prune(items: &ICoreWebView2ContextMenuItemCollection, devtools: bool) -> windows_core::Result<()> {
        let kinds = separators(items)?;
        // From the end, so the indices still to look at stay in place.
        for (i, &separator) in kinds.iter().enumerate().rev() {
            if separator {
                continue;
            }
            let mut name = PWSTR::null();
            // SAFETY: COM calls on the webview's thread; `take_pwstr` frees the name.
            unsafe { items.GetValueAtIndex(i as u32)?.Name(&mut name)? };
            if !allowed(&take_pwstr(name), devtools) {
                // SAFETY: as above.
                unsafe { items.RemoveValueAtIndex(i as u32)? };
            }
        }
        for i in super::dangling_separators(&separators(items)?) {
            // SAFETY: as above.
            unsafe { items.RemoveValueAtIndex(i as u32)? };
        }
        Ok(())
    }

    pub fn forget_site_data(pw: &PlatformWebview, what: super::SiteData) {
        let mut kinds = COREWEBVIEW2_BROWSING_DATA_KINDS_COOKIES;
        if let super::SiteData::CookiesAndCaches = what {
            for kind in [
                COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE,
                COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE,
                COREWEBVIEW2_BROWSING_DATA_KINDS_INDEXED_DB,
                COREWEBVIEW2_BROWSING_DATA_KINDS_WEB_SQL,
                COREWEBVIEW2_BROWSING_DATA_KINDS_FILE_SYSTEMS,
                COREWEBVIEW2_BROWSING_DATA_KINDS_SERVICE_WORKERS,
                COREWEBVIEW2_BROWSING_DATA_KINDS_BROWSING_HISTORY,
                COREWEBVIEW2_BROWSING_DATA_KINDS_DOWNLOAD_HISTORY,
                COREWEBVIEW2_BROWSING_DATA_KINDS_GENERAL_AUTOFILL,
                COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE,
                COREWEBVIEW2_BROWSING_DATA_KINDS_SETTINGS,
            ] {
                kinds |= kind;
            }
        }
        // SAFETY: WebView2 COM calls on the webview's own thread, where `with_webview` runs.
        let result = unsafe {
            (|| -> windows_core::Result<()> {
                let webview: ICoreWebView2_13 = pw.controller().CoreWebView2()?.cast()?;
                let profile: ICoreWebView2Profile2 = webview.Profile()?.cast()?;
                profile.ClearBrowsingData(
                    kinds,
                    &ClearBrowsingDataCompletedHandler::create(Box::new(|result| {
                        if let Err(e) = result {
                            eprintln!("could not clear the site data: {e}");
                        }
                        Ok(())
                    })),
                )
            })()
        };
        if let Err(e) = result {
            eprintln!("could not clear the site data: {e}");
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use tauri::webview::PlatformWebview;

    pub fn harden(_pw: &PlatformWebview, _devtools: bool) {}

    /// The main window of macOS never held the session: calls open in the default browser and
    /// the login window has a profile of its own.
    pub fn forget_site_data(_pw: &PlatformWebview, _what: super::SiteData) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kwin_script_finds_the_window_by_process_and_title() {
        let script = kwin_script(42, "(3) Мостик | \"x\"");
        assert!(
            script.starts_with("const title = \"(3) Мостик | \\\"x\\\"\";\n"),
            "{script}"
        );
        assert!(script.contains("w.pid !== 42"));
        assert!(script.contains("workspace.activeWindow = w"));
    }

    #[test]
    fn spelling_is_checked_in_russian_english_and_the_system_language() {
        assert_eq!(spelling_languages("en_US.UTF-8"), ["ru_RU", "en_US"]);
        assert_eq!(spelling_languages("ru_RU.UTF-8"), ["ru_RU", "en_US"]);
        assert_eq!(spelling_languages("de_DE.UTF-8"), ["ru_RU", "en_US", "de_DE"]);
        assert_eq!(spelling_languages("sr_RS@latin"), ["ru_RU", "en_US", "sr_RS"]);
        assert_eq!(spelling_languages("C.UTF-8"), ["ru_RU", "en_US"]);
        assert_eq!(spelling_languages(""), ["ru_RU", "en_US"]);
    }

    #[test]
    fn only_the_app_itself_is_the_app() {
        let app = |u: &str| is_app_url(&Url::parse(u).unwrap());
        assert!(app("tauri://localhost/"));
        assert!(app("tauri://localhost/index.html#ring"));
        assert!(app("http://tauri.localhost/"));
        assert!(app("https://tauri.localhost/index.html"));
        assert!(!app("https://example.org/"));
        assert!(!app("https://tauri.localhost.example.org/"));
        assert!(!app("tauri://evil/"));
        assert!(!app("file:///etc/passwd"));
        assert!(!app("about:blank"));
        assert!(!app("data:text/html,x"));
    }

    #[test]
    fn dangling_separators_go() {
        // | a | | b |  →  a | b
        assert_eq!(
            dangling_separators(&[true, false, true, true, false, true]),
            vec![5, 3, 0]
        );
        assert_eq!(dangling_separators(&[false, true, false]), Vec::<usize>::new());
        assert_eq!(dangling_separators(&[true, true]), vec![1, 0]);
        assert_eq!(dangling_separators(&[]), Vec::<usize>::new());
    }
}
