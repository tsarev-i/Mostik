//! Calls in their own window, like Slack huddles.
//!
//! The Telemost meeting page needs WebRTC. On Windows the WebView2 window of Tauri has it.
//! The WebKitGTK builds of Linux distributions do not (`RTCPeerConnection` is missing), so on
//! Linux Mostik starts an installed Chromium-based browser in app mode: a window without tabs
//! or an address bar, with a throwaway profile, driven over a DevTools pipe (`cdp.rs`). The
//! session cookies are put into that profile, so the user does not sign in again, and the
//! profile is deleted when the window closes. Without such a browser the call opens in the
//! default browser, as before.
//!
//! One call at a time, as in Slack. The UI learns about the current call from [`CALL_EVENT`]
//! and shows it at the bottom of the sidebar.

use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use mostik_yandex::session::CookieJar;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Url};
use tokio::sync::Mutex;

/// The current call changed: [`CallInfo`] or `null` when there is no call.
pub const CALL_EVENT: &str = "mostik://call";

/// Refusal to open a second call while one is open.
pub const BUSY: &str = "another call is already open, leave it first";

/// Hosts of the Telemost meeting pages.
const MEETING_HOSTS: [&str; 2] = ["telemost.360.yandex.ru", "telemost.yandex.ru"];

/// Initial size of the call window: compact, like the Slack huddle window.
const WIDTH: f64 = 960.0;
const HEIGHT: f64 = 680.0;

/// What the UI shows about the current call.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallInfo {
    /// The chat whose meeting it is; `None` for a meeting link from a message.
    pub chat_id: Option<String>,
    /// `#dev`, a person's name or "Telemost meeting".
    pub label: String,
}

struct Active {
    info: CallInfo,
    url: String,
    generation: u64,
    handle: imp::Handle,
}

/// The call window, if one is open.
#[derive(Default)]
pub struct CallState {
    /// Held while a window is being opened, so that two clicks do not open two windows.
    opening: Mutex<()>,
    active: StdMutex<Option<Active>>,
    generation: AtomicU64,
}

/// Whether the link is a Telemost meeting (`https://telemost.yandex.ru/j/…`).
pub fn is_meeting_url(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some_and(|h| MEETING_HOSTS.contains(&h))
            && u.path().starts_with("/j/")
            && u.path().len() > 3
    })
}

/// Opens the meeting in the call window. If that call is already open, its window comes to the
/// front; a different call is refused until the current one ends.
pub async fn open(app: &AppHandle, url: &str, info: CallInfo, jar: &CookieJar) -> Result<(), String> {
    let state = app.state::<CallState>();
    let _opening = state.opening.lock().await;
    if let Some((same, handle)) = current(app) {
        imp::focus(app, &handle).await;
        return if same == url { Ok(()) } else { Err(BUSY.into()) };
    }
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let title = window_title(app, &info.label);
    let launched = imp::launch(app, url, &title, jar, generation).await;
    if let Err(e) = &launched {
        eprintln!("call: {e}");
    }
    match launched? {
        Some(handle) => {
            if let Ok(mut a) = state.active.lock() {
                *a = Some(Active {
                    info: info.clone(),
                    url: url.to_owned(),
                    generation,
                    handle,
                });
            }
            let _ = app.emit(CALL_EVENT, Some(&info));
            Ok(())
        }
        None => crate::backend::open_external(url),
    }
}

/// Brings the call window to the front.
pub async fn focus(app: &AppHandle) {
    if let Some((_, handle)) = current(app) {
        imp::focus(app, &handle).await;
    }
}

/// Leaves the call: closes its window.
pub fn close(app: &AppHandle) {
    let active = app.state::<CallState>().active.lock().ok().and_then(|mut a| a.take());
    if let Some(active) = active {
        imp::close(app, &active.handle);
        let _ = app.emit(CALL_EVENT, None::<CallInfo>);
    }
}

/// The current call, if any: what the UI shows about it.
pub fn info(app: &AppHandle) -> Option<CallInfo> {
    current(app)?;
    let state = app.state::<CallState>();
    let active = state.active.lock().ok()?;
    active.as_ref().map(|a| a.info.clone())
}

/// Removes leftovers of earlier runs (profiles of the call browser). Called at startup.
pub fn cleanup(app: &AppHandle) {
    imp::cleanup(app);
}

/// URL and handle of the open call. A call whose window is gone is forgotten here.
fn current(app: &AppHandle) -> Option<(String, imp::Handle)> {
    let state = app.state::<CallState>();
    let mut active = state.active.lock().ok()?;
    let a = active.as_ref()?;
    if imp::alive(app, &a.handle) {
        return Some((a.url.clone(), a.handle.clone()));
    }
    *active = None;
    drop(active);
    let _ = app.emit(CALL_EVENT, None::<CallInfo>);
    None
}

/// The window of call `generation` has closed: forget the call, unless a newer one replaced it.
fn ended(app: &AppHandle, generation: u64) {
    let state = app.state::<CallState>();
    let Ok(mut active) = state.active.lock() else { return };
    if active.as_ref().is_some_and(|a| a.generation == generation) {
        *active = None;
        drop(active);
        let _ = app.emit(CALL_EVENT, None::<CallInfo>);
    }
}

fn window_title(app: &AppHandle, label: &str) -> String {
    format!("{} · {label}", crate::backend::text(app, "Созвон", "Call"))
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::{OsStr, OsString};
    use std::io::pipe;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::time::Duration;

    use mostik_yandex::session::CookieJar;
    use serde_json::{Value, json};
    use tauri::{AppHandle, Manager, Url};

    use super::{HEIGHT, MEETING_HOSTS, WIDTH};
    use tokio::sync::mpsc::UnboundedReceiver;

    use crate::cdp::{Cdp, Event};

    /// Lets the user pick the browser for calls, for example a Chromium build outside `PATH`.
    const BROWSER_ENV: &str = "MOSTIK_CALL_BROWSER";

    /// Chromium-based browsers, in order of preference. Firefox has no app mode and no pipe.
    const BROWSERS: [&str; 11] = [
        "chromium",
        "chromium-browser",
        "google-chrome-stable",
        "google-chrome",
        "yandex-browser-stable",
        "yandex-browser",
        "microsoft-edge-stable",
        "microsoft-edge",
        "brave-browser",
        "brave",
        "vivaldi-stable",
    ];

    const PROFILE_PREFIX: &str = "call-browser-";

    /// Start page of the app window: empty and dark, until the "connecting" page replaces it.
    const START_PAGE: &str = "data:text/html,%3Cmeta%20name=color-scheme%20content=dark%3E";

    #[derive(Clone)]
    pub struct Handle {
        cdp: Arc<Cdp>,
        target: String,
    }

    pub async fn launch(
        app: &AppHandle,
        url: &str,
        title: &str,
        jar: &CookieJar,
        generation: u64,
    ) -> Result<Option<Handle>, String> {
        let Some(browser) = find_browser() else {
            eprintln!("call: no Chromium-based browser found, opening the default browser");
            return Ok(None);
        };
        let profile = profile_dir(app, &browser, generation)?;
        // A tiny dark page first and the "connecting" page right after it: app mode needs a
        // real URL (`about:blank` gives an ordinary browser window), and Chromium names the
        // window's class after the start URL, so a whole page there would make it a page long.
        let (cdp, child) = match spawn(&browser, &profile, START_PAGE, &[]) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("call: could not start {}: {e}", browser.display());
                let _ = std::fs::remove_dir_all(&profile);
                return Ok(None);
            }
        };
        eprintln!("call: started {}", browser.display());
        let app2 = app.clone();
        let (cdp, events) = Cdp::start(cdp.0, cdp.1, move || {
            super::ended(&app2, generation);
            let mut child = child;
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&profile);
        });
        let loading = loading_page(app, title);
        match prepare(&cdp, url, title, &loading, jar).await {
            Ok(target) => {
                tauri::async_runtime::spawn(close_when_left(cdp.clone(), events, target.clone()));
                Ok(Some(Handle { cdp, target }))
            }
            Err(e) => {
                let _ = cdp.send("Browser.close", json!({}), None).await;
                Err(e)
            }
        }
    }

    pub async fn focus(_app: &AppHandle, h: &Handle) {
        let _ = h
            .cdp
            .send("Target.activateTarget", json!({ "targetId": h.target }), None)
            .await;
    }

    pub fn close(_app: &AppHandle, h: &Handle) {
        let cdp = h.cdp.clone();
        tauri::async_runtime::spawn(async move {
            let _ = cdp.send("Browser.close", json!({}), None).await;
        });
    }

    pub fn alive(_app: &AppHandle, h: &Handle) -> bool {
        !h.cdp.is_closed()
    }

    pub fn cleanup(app: &AppHandle) {
        let Ok(dir) = app.path().app_cache_dir() else { return };
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(PROFILE_PREFIX) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    /// The "connecting" page, cookies, camera and microphone, the window title, then the meeting.
    async fn prepare(cdp: &Cdp, url: &str, title: &str, loading: &str, jar: &CookieJar) -> Result<String, String> {
        let target = page_target(cdp).await?;
        let session = cdp
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": true }),
                None,
            )
            .await?;
        let session = session
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or("Target.attachToTarget: no sessionId")?;
        // Navigation events, for `close_when_left`.
        cdp.send("Page.enable", json!({}), Some(session)).await?;
        cdp.send("Page.navigate", json!({ "url": loading }), Some(session))
            .await?;
        cdp.send("Storage.setCookies", json!({ "cookies": cookies(jar) }), None)
            .await?;
        // The user has asked for the call, so the meeting page gets the camera and the
        // microphone without another prompt, as in a native app.
        for origin in origins(url) {
            cdp.send(
                "Browser.grantPermissions",
                json!({ "origin": origin, "permissions": ["audioCapture", "videoCapture"] }),
                None,
            )
            .await?;
        }
        cdp.send(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": title_script(title) }),
            Some(session),
        )
        .await?;
        cdp.send("Page.navigate", json!({ "url": url }), Some(session)).await?;
        Ok(target)
    }

    /// Closes the window once the user leaves the meeting, as the Slack huddle window does.
    /// After "Leave" Telemost opens its own chats in the same page; here that means the call is over.
    async fn close_when_left(cdp: Arc<Cdp>, mut events: UnboundedReceiver<Event>, target: String) {
        let mut in_meeting = false;
        while let Some(e) = events.recv().await {
            let Some(url) = main_frame_url(&e, &target) else {
                continue;
            };
            if super::is_meeting_url(url) {
                in_meeting = true;
            } else if in_meeting && !url.starts_with("data:") {
                eprintln!("call: left the meeting, closing the window");
                let _ = cdp.send("Browser.close", json!({}), None).await;
                return;
            }
        }
    }

    /// The new URL of the window's page, if the event is its navigation. The main frame has
    /// the target's id; `navigatedWithinDocument` covers the single-page app's own routing.
    pub(super) fn main_frame_url<'a>(e: &'a Event, target: &str) -> Option<&'a str> {
        let (frame, url) = match e.method.as_str() {
            "Page.frameNavigated" => {
                let f = e.params.get("frame")?;
                (f.get("id")?, f.get("url")?)
            }
            "Page.navigatedWithinDocument" => (e.params.get("frameId")?, e.params.get("url")?),
            _ => return None,
        };
        (frame.as_str()? == target).then_some(())?;
        url.as_str()
    }

    /// The app window's page. Right after the start the browser may not have created it yet.
    async fn page_target(cdp: &Cdp) -> Result<String, String> {
        for _ in 0..50 {
            let v = cdp.send("Target.getTargets", json!({}), None).await?;
            let page = v
                .get("targetInfos")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|t| matches!(t.get("type").and_then(Value::as_str), Some("page" | "app")))
                .and_then(|t| t.get("targetId")?.as_str());
            if let Some(id) = page {
                return Ok(id.to_owned());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("the call window did not open".into())
    }

    /// Starts the browser with the DevTools pipe on fds 3 (commands) and 4 (replies).
    pub(super) fn spawn(
        browser: &Path,
        profile: &Path,
        start_url: &str,
        extra: &[&str],
    ) -> std::io::Result<((std::io::PipeReader, std::io::PipeWriter), std::process::Child)> {
        let (child_in, to_child) = pipe()?;
        let (from_child, child_out) = pipe()?;
        let (fd_in, fd_out) = (child_in.as_raw_fd(), child_out.as_raw_fd());
        let mut cmd = Command::new(browser);
        cmd.arg(format!("--app={start_url}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--window-size={WIDTH},{HEIGHT}"))
            .args([
                "--remote-debugging-pipe",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-sync",
                // The profile is thrown away: no reason to ask KWallet or GNOME Keyring for a key.
                "--password-store=basic",
            ])
            .args(extra)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (key, value) in scrub_env(std::env::vars_os(), std::env::var_os("APPDIR").as_deref()) {
            match value {
                Some(v) => cmd.env(key, v),
                None => cmd.env_remove(key),
            };
        }
        // SAFETY: only async-signal-safe calls (fcntl, dup2) between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                // Move both ends above 4 first, so that one dup2 cannot overwrite the other.
                let a = libc::fcntl(fd_in, libc::F_DUPFD_CLOEXEC, 10);
                let b = libc::fcntl(fd_out, libc::F_DUPFD_CLOEXEC, 10);
                if a < 0 || b < 0 || libc::dup2(a, 3) < 0 || libc::dup2(b, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        // The child's ends stay open only in the child: then its exit closes the pipe.
        drop(child_in);
        drop(child_out);
        Ok(((from_child, to_child), child))
    }

    /// The browser from `MOSTIK_CALL_BROWSER`, or the first known one in `PATH`.
    pub(super) fn find_browser() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os(BROWSER_ENV).filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(p));
        }
        let path = std::env::var_os("PATH")?;
        BROWSERS.iter().find_map(|name| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|p| is_executable(p))
        })
    }

    fn is_executable(p: &Path) -> bool {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }

    /// A fresh profile directory, readable only by the user. A snap browser can write only to
    /// its own directory under `~/snap`, so its profile lives there.
    fn profile_dir(app: &AppHandle, browser: &Path, generation: u64) -> Result<PathBuf, String> {
        let name = format!("{PROFILE_PREFIX}{generation}");
        let dir = match snap_name(browser).zip(std::env::var_os("HOME")) {
            Some((snap, home)) => PathBuf::from(home).join("snap").join(snap).join("common").join(name),
            None => app
                .path()
                .app_cache_dir()
                .map_err(|e| format!("no cache directory: {e}"))?
                .join(name),
        };
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        Ok(dir)
    }

    /// Snap name of a browser started through `/snap/bin` (Ubuntu's Chromium).
    fn snap_name(browser: &Path) -> Option<String> {
        let name = browser.file_name()?.to_str()?;
        if browser.starts_with("/snap/bin") {
            return Some(name.split('.').next()?.to_owned());
        }
        // Ubuntu's `chromium-browser` package is a wrapper around the snap.
        (name == "chromium-browser" && Path::new("/snap/bin/chromium").exists()).then(|| "chromium".to_owned())
    }

    /// Session cookies in the `Storage.setCookies` format. Without an expiry they stay in memory
    /// and are never written to the profile. The jar keeps domains without the leading dot
    /// (`yandex.ru`), and the HTTP client matches them with subdomains, so here they are domain
    /// cookies too: `Session_id` of `yandex.ru` must reach `telemost.360.yandex.ru`. Yandex sets
    /// its session cookies with `SameSite=None`, and so do we.
    fn cookies(jar: &CookieJar) -> Vec<Value> {
        jar.cookies
            .iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "value": c.value,
                    "domain": format!(".{}", c.domain.trim_start_matches('.')),
                    "path": "/",
                    "secure": true,
                    "sameSite": "None",
                })
            })
            .collect()
    }

    /// Origins that get the camera and microphone: the meeting's own and both Telemost hosts.
    pub(super) fn origins(url: &str) -> Vec<String> {
        let mut out: Vec<String> = MEETING_HOSTS.iter().map(|h| format!("https://{h}")).collect();
        if let Ok(u) = Url::parse(url) {
            let own = u.origin().ascii_serialization();
            if !out.contains(&own) {
                out.insert(0, own);
            }
        }
        out
    }

    /// Keeps the window title ("Call · #dev") on the meeting page, which sets its own: writes to
    /// `document.title` are ignored, and a `<title>` the page changes directly is put back.
    fn title_script(title: &str) -> String {
        let t = serde_json::to_string(title).unwrap_or_else(|_| "\"\"".into());
        format!(
            r#"(() => {{
  if (window.top !== window) return;
  const t = {t};
  const own = Object.getOwnPropertyDescriptor(Document.prototype, "title");
  Object.defineProperty(document, "title", {{ configurable: true, get: () => own.get.call(document), set: () => {{}} }});
  const fix = () => {{ if (own.get.call(document) !== t) own.set.call(document, t); }};
  // Telemost may stay in `readyState = loading` for long, so `DOMContentLoaded` is no signal:
  // wait for `<head>` itself, then watch only it.
  const watch = () => {{
    if (!document.head) return false;
    fix();
    new MutationObserver(fix).observe(document.head, {{ subtree: true, childList: true, characterData: true }});
    return true;
  }};
  if (!watch()) {{
    const boot = new MutationObserver(() => {{ if (watch()) boot.disconnect(); }});
    boot.observe(document, {{ childList: true, subtree: true }});
  }}
}})();"#
        )
    }

    /// Dark "connecting" page shown until the meeting loads, in the colours of the Slack huddle window.
    fn loading_page(app: &AppHandle, title: &str) -> String {
        let connecting = crate::backend::text(app, "Подключаемся к созвону…", "Joining the call…");
        let html = format!(
            r##"<!doctype html><meta charset="utf-8"><title>{title}</title>
<style>
html,body{{margin:0;height:100%;background:#1a1d21;color:#d1d2d3;font:15px/1.4 Lato,"Noto Sans","Segoe UI",system-ui,sans-serif}}
body{{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px}}
.dot{{width:56px;height:56px;border-radius:50%;background:#2bac76;display:flex;align-items:center;justify-content:center;animation:p 1.6s ease-out infinite}}
@keyframes p{{0%{{box-shadow:0 0 0 0 rgba(43,172,118,.55)}}100%{{box-shadow:0 0 0 22px rgba(43,172,118,0)}}}}
b{{color:#f8f8f8;font-size:17px}}
</style>
<div class="dot"><svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"/></svg></div>
<b>{connecting}</b>"##,
            title = html_escape(title),
        );
        format!("data:text/html;charset=utf-8,{}", percent_encode(&html))
    }

    fn html_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    fn percent_encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len() * 3);
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    /// Environment fixes for the browser. Inside an AppImage, variables like `LD_LIBRARY_PATH`
    /// or `GIO_MODULE_DIR` point into the image, and its libraries would break the system
    /// browser: such entries are dropped, and the variable is removed if nothing is left.
    pub(super) fn scrub_env(
        vars: impl IntoIterator<Item = (OsString, OsString)>,
        appdir: Option<&OsStr>,
    ) -> Vec<(OsString, Option<OsString>)> {
        let Some(appdir) = appdir.and_then(OsStr::to_str).filter(|d| !d.is_empty()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (key, value) in vars {
            let Some(v) = value.to_str() else { continue };
            if !v.contains(appdir) {
                continue;
            }
            let kept: Vec<&str> = v.split(':').filter(|part| !part.contains(appdir)).collect();
            let kept = kept.join(":");
            out.push((key, (!kept.is_empty()).then(|| OsString::from(kept))));
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn vars(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
            list.iter().map(|(k, v)| (k.into(), v.into())).collect()
        }

        #[test]
        fn outside_an_appimage_nothing_changes() {
            assert!(scrub_env(vars(&[("LD_LIBRARY_PATH", "/opt/x")]), None).is_empty());
        }

        #[test]
        fn appimage_paths_are_dropped() {
            let dir = OsStr::new("/tmp/.mount_MostikAb");
            let fixed = scrub_env(
                vars(&[
                    ("LD_LIBRARY_PATH", "/tmp/.mount_MostikAb/usr/lib"),
                    ("XDG_DATA_DIRS", "/tmp/.mount_MostikAb/usr/share:/usr/share"),
                    ("HOME", "/home/u"),
                ]),
                Some(dir),
            );
            assert_eq!(
                fixed,
                vec![
                    (OsString::from("LD_LIBRARY_PATH"), None),
                    (OsString::from("XDG_DATA_DIRS"), Some(OsString::from("/usr/share"))),
                ]
            );
        }

        #[test]
        fn origins_include_the_meeting_host_once() {
            assert_eq!(
                origins("https://telemost.yandex.ru/j/123"),
                vec!["https://telemost.360.yandex.ru", "https://telemost.yandex.ru"]
            );
            assert_eq!(origins("https://example.org/j/1")[0], "https://example.org");
        }

        #[test]
        fn cookies_cover_subdomains() {
            use mostik_yandex::session::StoredCookie;
            let cookie = |domain: &str| StoredCookie {
                name: "Session_id".into(),
                value: "s".into(),
                domain: domain.into(),
            };
            let jar = CookieJar {
                cookies: vec![cookie("yandex.ru"), cookie(".yandex.ru")],
            };
            let c = cookies(&jar);
            assert_eq!(c[0]["domain"], ".yandex.ru");
            assert_eq!(c[1]["domain"], ".yandex.ru");
            assert_eq!(c[0]["sameSite"], "None");
        }

        #[test]
        fn navigation_of_the_main_frame_only() {
            let ev = |method: &str, params: Value| Event {
                method: method.into(),
                params,
                session: Some("S".into()),
            };
            let nav = ev(
                "Page.frameNavigated",
                json!({ "frame": { "id": "T", "url": "https://telemost.yandex.ru/" } }),
            );
            assert_eq!(main_frame_url(&nav, "T"), Some("https://telemost.yandex.ru/"));
            let iframe = ev(
                "Page.frameNavigated",
                json!({ "frame": { "id": "F", "parentId": "T", "url": "x" } }),
            );
            assert_eq!(main_frame_url(&iframe, "T"), None);
            let spa = ev(
                "Page.navigatedWithinDocument",
                json!({ "frameId": "T", "url": "https://telemost.yandex.ru/j/1" }),
            );
            assert_eq!(main_frame_url(&spa, "T"), Some("https://telemost.yandex.ru/j/1"));
            assert_eq!(main_frame_url(&ev("Page.loadEventFired", json!({})), "T"), None);
        }

        #[test]
        fn loading_page_is_a_single_data_url() {
            let s = percent_encode("<b>a b\n#&</b>");
            assert_eq!(s, "%3Cb%3Ea%20b%0A%23%26%3C%2Fb%3E");
        }

        /// Drives a real browser in headless mode: the pipe, cookies and permissions work, and
        /// WebRTC is there. Needs Chromium or Chrome and the network: `cargo test -- --ignored`.
        #[tokio::test]
        #[ignore]
        async fn real_browser_round_trip() {
            let Some(browser) = find_browser() else {
                eprintln!("no Chromium-based browser, skipped");
                return;
            };
            let profile = tempdir();
            let (pipe, mut child) = spawn(
                &browser,
                &profile,
                "about:blank",
                &[
                    "--headless=new",
                    "--use-fake-ui-for-media-stream",
                    "--use-fake-device-for-media-stream",
                ],
            )
            .unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let (cdp, mut events) = Cdp::start(pipe.0, pipe.1, move || {
                let _ = tx.send(());
            });
            let target = page_target(&cdp).await.unwrap();
            let jar = CookieJar {
                cookies: vec![mostik_yandex::session::StoredCookie {
                    name: "mostik_test".into(),
                    value: "1".into(),
                    domain: "yandex.ru".into(),
                }],
            };
            cdp.send("Storage.setCookies", json!({ "cookies": cookies(&jar) }), None)
                .await
                .unwrap();
            let s = cdp
                .send(
                    "Target.attachToTarget",
                    json!({ "targetId": target, "flatten": true }),
                    None,
                )
                .await
                .unwrap();
            let s = s["sessionId"].as_str().unwrap().to_owned();
            // The cookie of `yandex.ru` goes to the meeting host as well.
            let sent = cdp
                .send(
                    "Network.getCookies",
                    json!({ "urls": ["https://telemost.360.yandex.ru/j/1"] }),
                    Some(&s),
                )
                .await
                .unwrap();
            assert!(
                sent["cookies"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["name"] == "mostik_test")
            );
            let eval = |expr: &'static str| {
                let cdp = &cdp;
                let s = s.clone();
                async move {
                    cdp.send(
                        "Runtime.evaluate",
                        json!({ "expression": expr, "awaitPromise": true, "returnByValue": true }),
                        Some(&s),
                    )
                    .await
                    .unwrap()["result"]["value"]
                        .clone()
                }
            };
            assert_eq!(eval("typeof RTCPeerConnection").await, "function");
            // The title survives a page that sets its own, both ways.
            cdp.send("Page.enable", json!({}), Some(&s)).await.unwrap();
            cdp.send(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": title_script("Call · #dev") }),
                Some(&s),
            )
            .await
            .unwrap();
            let page = "data:text/html,<title>Telemost</title><script>document.title='In a call';\
                setTimeout(()=>{document.querySelector('title').textContent='Left'},50)</script>";
            cdp.send("Page.navigate", json!({ "url": page }), Some(&s))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(400)).await;
            assert_eq!(eval("document.title").await, "Call · #dev");
            // Its navigation came as an event of the main frame.
            let mut seen = false;
            while let Ok(e) = events.try_recv() {
                seen |= main_frame_url(&e, &target).is_some_and(|u| u.starts_with("data:text/html,<title>"));
            }
            assert!(seen, "no Page.frameNavigated for the main frame");
            cdp.send("Browser.close", json!({}), None).await.ok();
            rx.recv_timeout(Duration::from_secs(10)).expect("browser did not exit");
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&profile);
        }

        fn tempdir() -> PathBuf {
            let dir = std::env::temp_dir().join(format!("mostik-cdp-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use mostik_yandex::session::CookieJar;
    use tauri::webview::Color;
    use tauri::webview::cookie::Cookie;
    use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

    use super::{HEIGHT, WIDTH};

    #[derive(Clone)]
    pub struct Handle {
        label: String,
    }

    /// A WebView2 window: Chromium inside, so WebRTC works as in the browser.
    pub async fn launch(
        app: &AppHandle,
        url: &str,
        title: &str,
        jar: &CookieJar,
        generation: u64,
    ) -> Result<Option<Handle>, String> {
        let meeting: tauri::Url = url.parse().map_err(|e| format!("meeting URL: {e}"))?;
        let blank = "about:blank".parse().map_err(|e| format!("{e}"))?;
        // A closing window keeps its label for a while, so each call gets its own.
        let label = format!("call-{generation}");
        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(blank))
            .title(title)
            .inner_size(WIDTH, HEIGHT)
            .min_inner_size(480.0, 360.0)
            .background_color(Color(0x1a, 0x1d, 0x21, 0xff))
            .build()
            .map_err(|e| format!("could not open the call window: {e}"))?;
        crate::webview::harden(&window);
        let app2 = app.clone();
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                super::ended(&app2, generation);
            }
        });
        // The cookies go in before the meeting loads, so the page sees the session at once.
        for c in &jar.cookies {
            let cookie = Cookie::build((c.name.clone(), c.value.clone()))
                .domain(c.domain.clone())
                .path("/")
                .secure(true)
                .build();
            if let Err(e) = window.set_cookie(cookie) {
                eprintln!("call: could not set a cookie: {e}");
            }
        }
        window
            .navigate(meeting)
            .map_err(|e| format!("could not open the meeting: {e}"))?;
        Ok(Some(Handle { label }))
    }

    pub async fn focus(app: &AppHandle, h: &Handle) {
        if let Some(w) = app.get_webview_window(&h.label) {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }

    pub fn close(app: &AppHandle, h: &Handle) {
        if let Some(w) = app.get_webview_window(&h.label) {
            let _ = w.close();
        }
    }

    pub fn alive(app: &AppHandle, h: &Handle) -> bool {
        app.get_webview_window(&h.label).is_some()
    }

    pub fn cleanup(_app: &AppHandle) {}
}

/// Elsewhere (macOS) calls open in the default browser.
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod imp {
    use mostik_yandex::session::CookieJar;
    use tauri::AppHandle;

    #[derive(Clone)]
    pub struct Handle;

    pub async fn launch(
        _app: &AppHandle,
        _url: &str,
        _title: &str,
        _jar: &CookieJar,
        _generation: u64,
    ) -> Result<Option<Handle>, String> {
        Ok(None)
    }

    pub async fn focus(_app: &AppHandle, _h: &Handle) {}

    pub fn close(_app: &AppHandle, _h: &Handle) {}

    pub fn alive(_app: &AppHandle, _h: &Handle) -> bool {
        false
    }

    pub fn cleanup(_app: &AppHandle) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_links() {
        assert!(is_meeting_url("https://telemost.yandex.ru/j/12345678901234"));
        assert!(is_meeting_url("https://telemost.360.yandex.ru/j/12345678901234"));
        assert!(!is_meeting_url("https://telemost.yandex.ru/"));
        assert!(!is_meeting_url("https://telemost.yandex.ru/j/"));
        assert!(!is_meeting_url("http://telemost.yandex.ru/j/1"));
        assert!(!is_meeting_url("https://telemost.yandex.ru.evil.example/j/1"));
        assert!(!is_meeting_url("https://example.org/j/1"));
    }
}
