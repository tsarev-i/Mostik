//! Yandex ID login window.
//!
//! The user types the login, password and captcha on the Yandex page themselves.
//! When Yandex ID sends them back to `telemost.360.yandex.ru`, the session cookies are taken and the window closes.

use std::sync::{Arc, Mutex};

use mostik_yandex::endpoints::passport_login_url;
use mostik_yandex::session::{CookieJar, StoredCookie};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::oneshot;

/// Window label of the login window.
pub const LABEL: &str = "login";
const RETURN_HOST: &str = "telemost.360.yandex.ru";

/// Opens the login window. The channel closes without a value if the user closes the window.
pub fn open(app: &AppHandle) -> Result<oneshot::Receiver<CookieJar>, String> {
    if let Some(existing) = app.get_webview_window(LABEL) {
        let _ = existing.close();
    }
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let url = passport_login_url().parse().map_err(|e| format!("login URL: {e}"))?;
    let on_close = tx.clone();
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .title(crate::backend::text(
            app,
            "Вход через Яндекс ID | Мостик (неофициальный клиент)",
            "Sign in with Yandex ID | Mostik (unofficial client)",
        ))
        .inner_size(540.0, 760.0)
        .on_page_load(move |win, payload| {
            if payload.event() != PageLoadEvent::Finished || payload.url().host_str() != Some(RETURN_HOST) {
                return;
            }
            let tx = tx.clone();
            // Cookies are read outside the window event handler.
            tauri::async_runtime::spawn(async move {
                let Ok(cookies) = win.cookies() else { return };
                let jar = CookieJar::from_cookies(cookies.into_iter().map(|c| StoredCookie {
                    name: c.name().to_owned(),
                    value: c.value().to_owned(),
                    domain: c.domain().unwrap_or(RETURN_HOST).to_owned(),
                }));
                if !jar.has_session() {
                    return;
                }
                if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                    let _ = tx.send(jar);
                }
                let _ = win.close();
            });
        })
        .build()
        .map_err(|e| format!("could not open the login window: {e}"))?;
    crate::webview::harden(&window);
    // The window was closed before signing in: drop the sender so that `login` reports "sign-in cancelled".
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            drop(on_close.lock().ok().and_then(|mut t| t.take()));
        }
    });
    Ok(rx)
}
