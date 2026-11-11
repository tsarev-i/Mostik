//! Yandex ID login window.
//!
//! The user types the login, password and captcha on the Yandex page themselves.
//! When Yandex ID sends them back to `telemost.360.yandex.ru`, the session cookies are taken and the window closes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use mostik_yandex::endpoints::passport_login_url;
use mostik_yandex::session::{CookieJar, StoredCookie};
use serde_json::{Value, json};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::oneshot;

/// Label prefix of login windows; each window gets its own number.
const LABEL_PREFIX: &str = "login-";
const RETURN_HOST: &str = "telemost.360.yandex.ru";

static NEXT_WINDOW: AtomicU32 = AtomicU32::new(1);

/// The way through the login window, rewritten to `probe-login.json` on every step: page
/// addresses without the query (it may carry tokens), load errors and the cookie names on
/// return, never their values. Sign-in problems on other machines are read from it.
#[derive(Clone)]
struct Trace {
    app: AppHandle,
    steps: Arc<Mutex<Vec<Value>>>,
}

impl Trace {
    fn new(app: &AppHandle) -> Self {
        let trace = Self {
            app: app.clone(),
            steps: Arc::default(),
        };
        trace.note(json!({ "opened": mostik_yandex::identity::ua_suffix() }));
        trace
    }

    fn note(&self, step: Value) {
        if let Ok(mut steps) = self.steps.lock() {
            steps.push(step);
            crate::backend::write_probe(&self.app, "probe-login.json", &Value::Array(steps.clone()));
        }
    }
}

/// Scheme, host and path of a page: the query and the fragment may carry tokens.
fn bare(url: &Url) -> String {
    format!("{}://{}{}", url.scheme(), url.host_str().unwrap_or(""), url.path())
}

/// Opens the login window. The channel closes without a value if the user closes the window.
pub fn open(app: &AppHandle) -> Result<oneshot::Receiver<CookieJar>, String> {
    // Closing is asynchronous, so a new window cannot reuse the old label: it gets a fresh one.
    for (label, window) in app.webview_windows() {
        if label.starts_with(LABEL_PREFIX) {
            let _ = window.close();
        }
    }
    let label = format!("{LABEL_PREFIX}{}", NEXT_WINDOW.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let url = passport_login_url().parse().map_err(|e| format!("login URL: {e}"))?;
    let on_close = tx.clone();
    let trace = Trace::new(app);
    let on_load = trace.clone();
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        // A clean store every time: the session the app uses lives in the password store, and
        // cookies left by an earlier account (or by a sign-out on the Yandex page) would make
        // Yandex ID return the old account or no session at all instead of the login form.
        .incognito(true)
        .title(crate::backend::text(
            app,
            "Вход через Яндекс ID | Мостик (неофициальный клиент)",
            "Sign in with Yandex ID | Mostik (unofficial client)",
        ))
        .inner_size(540.0, 760.0)
        .on_page_load(move |win, payload| {
            let finished = payload.event() == PageLoadEvent::Finished;
            on_load.note(json!({ if finished { "finished" } else { "started" }: bare(payload.url()) }));
            if !finished || payload.url().host_str() != Some(RETURN_HOST) {
                return;
            }
            let (tx, trace) = (tx.clone(), on_load.clone());
            // Cookies are read outside the window event handler.
            tauri::async_runtime::spawn(async move {
                let cookies = match win.cookies() {
                    Ok(cookies) => cookies,
                    Err(e) => return trace.note(json!({ "cookies_error": e.to_string() })),
                };
                let jar = CookieJar::from_cookies(cookies.into_iter().map(|c| StoredCookie {
                    name: c.name().to_owned(),
                    value: c.value().to_owned(),
                    domain: c.domain().unwrap_or(RETURN_HOST).to_owned(),
                }));
                let names: Vec<String> = jar.cookies.iter().map(|c| format!("{}@{}", c.name, c.domain)).collect();
                trace.note(json!({ "cookies": names, "session": jar.has_session() }));
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
    retry_failed_loads(&window, trace);
    // The window was closed before signing in: drop the sender so that `login` reports "sign-in cancelled".
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            drop(on_close.lock().ok().and_then(|mut t| t.take()));
        }
    });
    Ok(rx)
}

/// A page of the login window that fails to load, for example over a connection the server has
/// already dropped, is loaded again: the window has no reload button, and the WebKit error page
/// would be a dead end. After a few attempts the error page shows.
#[cfg(target_os = "linux")]
fn retry_failed_loads(window: &tauri::WebviewWindow, trace: Trace) {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;
    use webkit2gtk::{NetworkError, PolicyError, WebViewExt, glib};

    const RETRIES: u32 = 2;

    let _ = window.with_webview(move |pw| {
        let left = Rc::new(Cell::new(RETRIES));
        let view = pw.inner();
        let crashed = trace.clone();
        view.connect_web_process_terminated(move |_, reason| {
            crashed.note(json!({ "web_process_terminated": format!("{reason:?}") }));
        });
        view.connect_load_failed(move |view, _, uri, error| {
            // A navigation stopped on purpose (a new page, a closed window) is no failure.
            let stopped =
                error.kind::<NetworkError>() == Some(NetworkError::Cancelled) || error.kind::<PolicyError>().is_some();
            let retry = !stopped && left.get() > 0;
            let page = Url::parse(uri).map(|u| bare(&u)).unwrap_or_default();
            trace.note(json!({ "failed": page, "error": error.to_string(), "retry": retry }));
            if !retry {
                return false;
            }
            left.set(left.get() - 1);
            let (view, uri) = (view.clone(), uri.to_owned());
            glib::timeout_add_local_once(Duration::from_secs(1), move || view.load_uri(&uri));
            true
        });
    });
}

#[cfg(not(target_os = "linux"))]
fn retry_failed_loads(_window: &tauri::WebviewWindow, _trace: Trace) {}
