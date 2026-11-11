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

// Elsewhere (macOS) calls open in the default browser, so the window and end-of-call helpers
// are used only by the Linux and Windows `imp` (and by the tests).
#![cfg_attr(not(any(target_os = "linux", target_os = "windows")), allow(dead_code))]

use std::future::Future;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
    /// Calls opened so far, in the window or in the default browser.
    opens: AtomicU64,
}

fn is_meeting_host(u: &Url) -> bool {
    u.host_str().is_some_and(|h| MEETING_HOSTS.contains(&h))
}

/// Whether the link is a Telemost meeting (`https://telemost.yandex.ru/j/…`, or the bare
/// meeting page `/private-join/…`).
pub fn is_meeting_url(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && is_meeting_host(&u)
            && ["/j/", "/private-join/"]
                .iter()
                .any(|p| u.path().strip_prefix(p).is_some_and(|id| !id.is_empty()))
    })
}

/// Cookie domain as it reaches the call window: a domain cookie (`.yandex.ru`), so that
/// `Session_id` of `yandex.ru` also goes to `telemost.360.yandex.ru`. The jar keeps the domain
/// without the dot, and the HTTP client matches subdomains too.
fn cookie_domain(domain: &str) -> String {
    format!(".{}", domain.trim_start_matches('.'))
}

/// A `__Host-` cookie must stay on its own host: browsers refuse it with a `Domain`.
fn host_only(name: &str) -> bool {
    name.starts_with("__Host-")
}

/// How long the meeting page must be open before leaving it counts as the end of the call.
/// Redirects right after the load (language, sign-in, the server's own routing) are not "Leave".
const ARM_AFTER: Duration = Duration::from_secs(5);

/// Watches the call window's navigations for the end of the call, the same way on Linux (CDP
/// events) and Windows (`on_navigation`).
#[derive(Default)]
struct EndWatch {
    /// When the window first reached the meeting page.
    meeting_since: Option<Instant>,
}

impl EndWatch {
    /// Whether the window going to `url` at `now` ends the call. `same_document` marks the page
    /// changing its own address (the single-page app's routing) rather than loading a new one.
    ///
    /// The page script's [`ENDED_URL`] always does. Leaving the meeting page for another Yandex
    /// page does too: after "Leave" Telemost shows its chats or home page there. The page's own
    /// routing counts at once; a page load only once the meeting page has been open for
    /// [`ARM_AFTER`], since loads right after it are redirects. Sign-in pages never end the
    /// call, nor do pages outside Yandex (a corporate sign-in): the user may sign in right there.
    fn ends(&mut self, url: &str, now: Instant, same_document: bool) -> bool {
        if url == ENDED_URL {
            return true;
        }
        if is_meeting_url(url) {
            self.meeting_since.get_or_insert(now);
            return false;
        }
        let Some(since) = self.meeting_since else { return false };
        let armed = same_document || now.duration_since(since) >= ARM_AFTER;
        armed && Url::parse(url).is_ok_and(|u| u.scheme() == "https" && is_yandex(&u) && !is_sign_in(&u))
    }
}

fn is_yandex(u: &Url) -> bool {
    u.host_str().is_some_and(|h| {
        ["yandex.ru", "yandex.com", "yandex.net"]
            .iter()
            .any(|d| h == *d || h.ends_with(&format!(".{d}")))
    })
}

/// Yandex ID and other sign-in pages: `passport.yandex.ru` and its regional twins, `id.`,
/// `oauth.`, `sso.` and `auth.` hosts.
fn is_sign_in(u: &Url) -> bool {
    u.host_str().is_some_and(|h| {
        h.contains(".passport.")
            || ["passport.", "id.", "oauth.", "sso.", "auth."]
                .iter()
                .any(|p| h.starts_with(p))
    })
}

/// Opens the meeting in the call window. If that call is already open, its window comes to the
/// front; a different call is refused until the current one ends.
pub async fn open(app: &AppHandle, url: &str, info: CallInfo, jar: &CookieJar) -> Result<(), String> {
    let state = app.state::<CallState>();
    let _opening = state.opening.lock().await;
    if let Some((current, handle)) = current(app) {
        imp::focus(app, &handle).await;
        return if current.url == url { Ok(()) } else { Err(BUSY.into()) };
    }
    open_locked(app, url, info, jar).await
}

/// Opens the call of a chat. `prepare` asks Telemost for its link (`create_meeting`, which also
/// announces the call in the chat) and gives the session cookies; it runs only when no call is
/// open. An open call of this chat only comes to the front, and so does a second click while the
/// window is still opening: the check and the request run under the same lock as the opening.
/// Any other open call, including one from a message link, is refused before `prepare`, so
/// no chat hears of a call that does not open.
pub async fn open_chat_call<F, Fut>(app: &AppHandle, info: CallInfo, prepare: F) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(String, CookieJar), String>>,
{
    let state = app.state::<CallState>();
    let _opening = state.opening.lock().await;
    if let Some((current, handle)) = current(app) {
        imp::focus(app, &handle).await;
        return if current.info.chat_id.is_some() && current.info.chat_id == info.chat_id {
            Ok(())
        } else {
            Err(BUSY.into())
        };
    }
    let (url, jar) = prepare().await?;
    open_locked(app, &url, info, &jar).await
}

/// The opening itself; the caller holds `CallState::opening` and has seen no open call.
async fn open_locked(app: &AppHandle, url: &str, info: CallInfo, jar: &CookieJar) -> Result<(), String> {
    let state = app.state::<CallState>();
    state.opens.fetch_add(1, Ordering::SeqCst);
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let title = window_title(app, &info.label);
    let english = crate::backend::text(app, "ru", "en") == "en";
    let page = meeting_page(url, english);
    let launched = imp::launch(app, &page, &title, jar, generation).await;
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

/// Calls opened so far (in the window or in the default browser): a status clean-up scheduled
/// before a newer call must leave that call's status alone.
pub fn opens(app: &AppHandle) -> u64 {
    app.state::<CallState>().opens.load(Ordering::SeqCst)
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
        crate::backend::clear_meeting_status_later(app.clone());
    }
}

/// The current call, if any: what the UI shows about it.
pub fn info(app: &AppHandle) -> Option<CallInfo> {
    current(app).map(|(c, _)| c.info)
}

/// Removes leftovers of earlier runs (profiles of the call browser). Called at startup.
pub fn cleanup(app: &AppHandle) {
    imp::cleanup(app);
}

/// What is known about the open call.
struct Current {
    info: CallInfo,
    url: String,
}

/// The open call and its window handle; `None` once its window is gone. Forgetting the call is
/// up to `ended`, which runs when the window goes on both platforms, so that the status check
/// after the call is scheduled once.
fn current(app: &AppHandle) -> Option<(Current, imp::Handle)> {
    let state = app.state::<CallState>();
    let active = state.active.lock().ok()?;
    let a = active.as_ref()?;
    imp::alive(app, &a.handle).then(|| {
        let current = Current {
            info: a.info.clone(),
            url: a.url.clone(),
        };
        (current, a.handle.clone())
    })
}

/// The window of call `generation` has closed: forget the call, unless a newer one replaced it.
fn ended(app: &AppHandle, generation: u64) {
    let state = app.state::<CallState>();
    let Ok(mut active) = state.active.lock() else { return };
    if active.as_ref().is_some_and(|a| a.generation == generation) {
        *active = None;
        drop(active);
        let _ = app.emit(CALL_EVENT, None::<CallInfo>);
        crate::backend::clear_meeting_status_later(app.clone());
    }
}

/// Where the page script sends the window once the call is over; the window then closes.
const ENDED_URL: &str = "about:blank#mostik-call-ended";

/// The meeting page for the call window. On `telemost.360.yandex.ru` the `/j/<id>` link opens the
/// whole messenger (Yandex 360 bar, chats, notifications) with the meeting in an iframe; the call
/// window shows only that iframe's page, `/private-join/<id>`, as its own. Other links stay as is.
pub fn meeting_page(url: &str, english: bool) -> String {
    let Ok(u) = Url::parse(url) else { return url.to_owned() };
    let id = u
        .path()
        .strip_prefix("/j/")
        .filter(|id| !id.is_empty() && !id.contains('/'));
    match (u.host_str(), id) {
        (Some(host @ "telemost.360.yandex.ru"), Some(id)) => format!(
            "https://{host}/private-join/{id}?theme=dark&preview_theme=dark&lang={}",
            if english { "en" } else { "ru" }
        ),
        _ => url.to_owned(),
    }
}

/// Runs in the meeting page before its own scripts.
///
/// It keeps the window title ("Call · #dev"): writes to `document.title` are ignored, and a
/// `<title>` the page changes directly is put back.
///
/// It also notices the end of the call. Opened on its own, the meeting page does not navigate
/// anywhere after "Leave": it just goes blank. The "×" of the join screen does nothing at all, so
/// a click on it ends the call right away. So once the page
/// has shown something, has no WebRTC connection left and stays blank for two seconds, it goes
/// to [`ENDED_URL`] and the window closes. While any connection is open the window stays, even
/// if the page hides all its controls.
fn page_script(title: &str) -> String {
    let t = serde_json::to_string(title).unwrap_or_else(|_| "\"\"".into());
    let ended = serde_json::to_string(ENDED_URL).unwrap_or_default();
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

  // The "×" of the join screen leads to the messenger's home screen, which the window does
  // not have: here it closes the window.
  document.addEventListener("click", e => {{
    if (e.target instanceof Element && e.target.closest('[data-testid="to-home-screen-button"]')) {{
      e.stopPropagation();
      location.replace({ended});
    }}
  }}, true);

  const Native = window.RTCPeerConnection;
  if (!Native) return;
  const open = new Set();
  window.RTCPeerConnection = class RTCPeerConnection extends Native {{
    constructor(...args) {{
      super(...args);
      open.add(this);
      this.addEventListener("connectionstatechange", () => {{ if (this.connectionState === "closed") open.delete(this); }});
    }}
    close() {{
      open.delete(this);
      return super.close();
    }}
  }};
  let shown = false, blankSince = 0;
  setInterval(() => {{
    if (open.size) {{ shown = true; blankSince = 0; return; }}
    const blank = !document.body || !document.body.innerText.trim();
    if (!blank) {{ shown = true; blankSince = 0; return; }}
    if (!shown) return;
    blankSince = blankSince || Date.now();
    if (Date.now() - blankSince > 2000) location.replace({ended});
  }}, 500);
}})();"#
    )
}

fn window_title(app: &AppHandle, label: &str) -> String {
    format!("{} · {label}", crate::backend::text(app, "Созвон", "Call"))
}

#[cfg(target_os = "linux")]
mod imp {
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
        cleanup(app);
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
            dispose(child, &profile);
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

    /// Removes call profiles left by Mostik processes that are gone (a crash, a killed process).
    /// Profiles of another running Mostik are its own business; this process has no call open
    /// when it calls this, so its own leftovers go too.
    pub fn cleanup(app: &AppHandle) {
        for dir in profile_parents(app) {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let name = e.file_name();
                let Some(owner) = name.to_str().and_then(profile_owner) else {
                    continue;
                };
                let alive = Path::new("/proc").join(owner.to_string()).exists();
                if owner == std::process::id() || !alive {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
    }

    /// Where call profiles live: the app cache, and `~/snap/<name>/common` for snap browsers.
    fn profile_parents(app: &AppHandle) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = app.path().app_cache_dir().into_iter().collect();
        if let Some(snaps) = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("snap"))
            && let Ok(entries) = std::fs::read_dir(snaps)
        {
            dirs.extend(entries.flatten().map(|e| e.path().join("common")));
        }
        dirs
    }

    /// The Mostik process a profile directory (`call-browser-<pid>-<n>`) belongs to.
    pub(super) fn profile_owner(name: &str) -> Option<u32> {
        name.strip_prefix(PROFILE_PREFIX)?.split_once('-')?.0.parse().ok()
    }

    /// The browser has closed the pipe: it is exiting. Its helper processes (network, storage,
    /// GPU) may still write to the profile for a moment, so the whole process group goes first,
    /// then the profile.
    fn dispose(mut child: std::process::Child, profile: &Path) {
        // Before `wait`: until the leader is reaped its pid, and so the group id, cannot be reused,
        // so the signal cannot reach an unrelated group.
        // SAFETY: plain syscall; the group was created for this browser by `process_group(0)`.
        unsafe {
            libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
        }
        let _ = child.wait();
        for _ in 0..5 {
            std::thread::sleep(Duration::from_millis(200));
            match std::fs::remove_dir_all(profile) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => continue,
                _ if profile.exists() => continue,
                _ => return,
            }
        }
        eprintln!("call: could not remove {}", profile.display());
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
            json!({ "source": super::page_script(title) }),
            Some(session),
        )
        .await?;
        cdp.send("Page.navigate", json!({ "url": url }), Some(session)).await?;
        Ok(target)
    }

    /// Closes the window once the user leaves the meeting, as the Slack huddle window does
    /// (see `EndWatch`).
    async fn close_when_left(cdp: Arc<Cdp>, mut events: UnboundedReceiver<Event>, target: String) {
        let mut watch = super::EndWatch::default();
        while let Some(e) = events.recv().await {
            let Some(url) = main_frame_url(&e, &target) else {
                continue;
            };
            let same_document = e.method == "Page.navigatedWithinDocument";
            if watch.ends(&url, std::time::Instant::now(), same_document) {
                eprintln!("call: left the meeting, closing the window");
                let _ = cdp.send("Browser.close", json!({}), None).await;
                return;
            }
        }
    }

    /// The new URL of the window's page, if the event is its navigation. The main frame has
    /// the target's id; `navigatedWithinDocument` covers the single-page app's own routing.
    /// `frameNavigated` keeps the fragment apart (`urlFragment`): it goes back on, since
    /// `ENDED_URL` is told apart by it.
    pub(super) fn main_frame_url(e: &Event, target: &str) -> Option<String> {
        let (frame, url, fragment) = match e.method.as_str() {
            "Page.frameNavigated" => {
                let f = e.params.get("frame")?;
                (
                    f.get("id")?,
                    f.get("url")?,
                    f.get("urlFragment").and_then(Value::as_str),
                )
            }
            "Page.navigatedWithinDocument" => (e.params.get("frameId")?, e.params.get("url")?, None),
            _ => return None,
        };
        (frame.as_str()? == target).then_some(())?;
        Some(format!("{}{}", url.as_str()?, fragment.unwrap_or_default()))
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
                // No "Translate this page?" over the call.
                "--disable-features=Translate",
            ])
            .args(extra)
            // Its own process group, so that `dispose` can stop the helpers of a crashed browser.
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // From inside an AppImage the image's libraries would break the system browser.
        crate::appimage::outside(&mut cmd);
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
        // The pid keeps two running Mostik processes (a release and a dev build) apart.
        let name = format!("{PROFILE_PREFIX}{}-{generation}", std::process::id());
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
                let mut v = json!({
                    "name": c.name,
                    "value": c.value,
                    "path": "/",
                    "secure": true,
                    "sameSite": "None",
                });
                if super::host_only(&c.name) {
                    v["url"] = json!(format!("https://{}/", c.domain.trim_start_matches('.')));
                } else {
                    v["domain"] = json!(super::cookie_domain(&c.domain));
                }
                v
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn origins_include_the_meeting_host_once() {
            assert_eq!(
                origins("https://telemost.yandex.ru/j/123"),
                vec!["https://telemost.360.yandex.ru", "https://telemost.yandex.ru"]
            );
            assert_eq!(origins("https://example.org/j/1")[0], "https://example.org");
        }

        #[test]
        fn host_prefixed_cookies_stay_on_their_host() {
            use mostik_yandex::session::StoredCookie;
            let jar = CookieJar {
                cookies: vec![StoredCookie {
                    name: "__Host-x".into(),
                    value: "1".into(),
                    domain: "telemost.360.yandex.ru".into(),
                }],
            };
            let c = cookies(&jar);
            assert_eq!(c[0]["url"], "https://telemost.360.yandex.ru/");
            assert!(c[0].get("domain").is_none());
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
        fn profile_owner_is_the_pid_in_the_name() {
            assert_eq!(profile_owner("call-browser-4242-3"), Some(4242));
            assert_eq!(profile_owner("call-browser-3"), None);
            assert_eq!(profile_owner("other-4242-3"), None);
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
            assert_eq!(
                main_frame_url(&nav, "T").as_deref(),
                Some("https://telemost.yandex.ru/")
            );
            let iframe = ev(
                "Page.frameNavigated",
                json!({ "frame": { "id": "F", "parentId": "T", "url": "x" } }),
            );
            assert_eq!(main_frame_url(&iframe, "T"), None);
            let ended = ev(
                "Page.frameNavigated",
                json!({ "frame": { "id": "T", "url": "about:blank", "urlFragment": "#mostik-call-ended" } }),
            );
            assert_eq!(main_frame_url(&ended, "T").as_deref(), Some(super::super::ENDED_URL));
            let spa = ev(
                "Page.navigatedWithinDocument",
                json!({ "frameId": "T", "url": "https://telemost.yandex.ru/j/1" }),
            );
            assert_eq!(
                main_frame_url(&spa, "T").as_deref(),
                Some("https://telemost.yandex.ru/j/1")
            );
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
                json!({ "source": super::super::page_script("Call · #dev") }),
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
            // A page that showed something and went blank, with no WebRTC connection: the call is over.
            let page =
                "data:text/html,<p>In a call</p><script>setTimeout(()=>{document.body.textContent=''},1200)</script>";
            cdp.send("Page.navigate", json!({ "url": page }), Some(&s))
                .await
                .unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            let mut ended = false;
            while !ended && tokio::time::Instant::now() < deadline {
                if let Ok(Some(e)) = tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
                    ended = main_frame_url(&e, &target).as_deref() == Some(super::super::ENDED_URL);
                }
            }
            assert!(ended, "the blank page did not report the end of the call");
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
    use std::sync::Mutex;

    use mostik_yandex::session::CookieJar;
    use tauri::webview::Color;
    use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
    use webview2_com::Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE, ICoreWebView2_2};
    use windows_core::{HSTRING, Interface};

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
        let app2 = app.clone();
        let label2 = label.clone();
        let watch = Mutex::new(super::EndWatch::default());
        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(blank))
            .title(title)
            .initialization_script(super::page_script(title))
            // The end of the call, as on Linux (`EndWatch`). Routing inside the page does not
            // come here; on the bare meeting page the page script covers it with `ENDED_URL`.
            .on_navigation(move |url| {
                let ends = watch
                    .lock()
                    .is_ok_and(|mut w| w.ends(url.as_str(), std::time::Instant::now(), false));
                if !ends {
                    return true;
                }
                // Not from inside the navigation handler: close once it has returned.
                if let Some(w) = app2.get_webview_window(&label2) {
                    tauri::async_runtime::spawn(async move {
                        let _ = w.close();
                    });
                }
                false
            })
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
        if let Err(e) = set_cookies(&window, jar).await {
            eprintln!("call: could not set the session cookies: {e}");
        }
        window
            .navigate(meeting)
            .map_err(|e| format!("could not open the meeting: {e}"))?;
        Ok(Some(Handle { label }))
    }

    /// Puts the session cookies into the WebView2 profile as domain cookies (`.yandex.ru`), as on
    /// Linux. Tauri's `set_cookie` cannot: the `cookie` crate hands out the domain without its
    /// leading dot, and to WebView2 `yandex.ru` without the dot means that host only, so
    /// `telemost.360.yandex.ru` would not get `Session_id`.
    async fn set_cookies(window: &WebviewWindow, jar: &CookieJar) -> Result<(), String> {
        let cookies: Vec<(String, String, String)> = jar
            .cookies
            .iter()
            .map(|c| {
                let domain = if super::host_only(&c.name) {
                    c.domain.trim_start_matches('.').to_owned()
                } else {
                    super::cookie_domain(&c.domain)
                };
                (c.name.clone(), c.value.clone(), domain)
            })
            .collect();
        let (tx, rx) = tokio::sync::oneshot::channel();
        window
            .with_webview(move |pw| {
                // SAFETY: WebView2 COM calls on the webview's own thread, where `with_webview` runs.
                let result = unsafe {
                    (|| -> windows_core::Result<()> {
                        let webview: ICoreWebView2_2 = pw.controller().CoreWebView2()?.cast()?;
                        let manager = webview.CookieManager()?;
                        // One cookie WebView2 refuses must not cost the others, `Session_id` among them.
                        for (name, value, domain) in &cookies {
                            let added = (|| -> windows_core::Result<()> {
                                let cookie = manager.CreateCookie(
                                    &HSTRING::from(name),
                                    &HSTRING::from(value),
                                    &HSTRING::from(domain),
                                    &HSTRING::from("/"),
                                )?;
                                cookie.SetIsSecure(true)?;
                                cookie.SetSameSite(COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE)?;
                                manager.AddOrUpdateCookie(&cookie)
                            })();
                            if let Err(e) = added {
                                eprintln!("call: cookie {name} for {domain} not set: {e}");
                            }
                        }
                        Ok(())
                    })()
                };
                let _ = tx.send(result.map_err(|e| e.to_string()));
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "the call window went away".to_owned())?
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
        assert!(is_meeting_url(
            "https://telemost.360.yandex.ru/private-join/1?theme=dark"
        ));
        assert!(!is_meeting_url("https://telemost.360.yandex.ru/private-join/"));
    }

    #[test]
    fn what_ends_the_call() {
        let t0 = Instant::now();
        let soon = t0 + Duration::from_secs(1);
        let later = t0 + ARM_AFTER;
        let meeting = "https://telemost.360.yandex.ru/private-join/123?theme=dark";
        let in_meeting = || {
            let mut w = EndWatch::default();
            assert!(!w.ends(meeting, t0, false));
            w
        };
        // The page script's signal, at any time.
        assert!(EndWatch::default().ends(ENDED_URL, t0, false));
        // A page load right after the meeting page is a redirect, not "Leave".
        assert!(!in_meeting().ends("https://telemost.360.yandex.ru/?retpath=x", soon, false));
        // The page's own routing away from the meeting is "Leave", however soon.
        assert!(in_meeting().ends("https://telemost.yandex.ru/chats", soon, true));
        // Later, loading Telemost's or another Yandex page ends it too.
        assert!(in_meeting().ends("https://telemost.yandex.ru/chats", later, false));
        assert!(in_meeting().ends("https://360.yandex.ru/", later, false));
        // Sign-in pages and pages outside Yandex do not, nor does staying in the meeting.
        assert!(!in_meeting().ends("https://passport.yandex.ru/auth?retpath=x", later, false));
        assert!(!in_meeting().ends("https://oauth.yandex.ru/authorize?x", later, true));
        assert!(!in_meeting().ends("https://id.yandex.ru/", later, false));
        assert!(!in_meeting().ends("https://login.example-corp.com/saml", later, false));
        assert!(!in_meeting().ends("https://telemost.360.yandex.ru/j/123", later, true));
        // Before the meeting page, nothing but the signal counts.
        assert!(!EndWatch::default().ends("https://telemost.360.yandex.ru/", later, true));
        assert!(!EndWatch::default().ends("about:blank", later, false));
    }

    #[test]
    fn the_window_shows_the_bare_meeting_page() {
        assert_eq!(
            meeting_page("https://telemost.360.yandex.ru/j/123", false),
            "https://telemost.360.yandex.ru/private-join/123?theme=dark&preview_theme=dark&lang=ru"
        );
        assert!(meeting_page("https://telemost.360.yandex.ru/j/123", true).ends_with("lang=en"));
        // Personal Telemost has no messenger around the meeting: the link stays.
        assert_eq!(
            meeting_page("https://telemost.yandex.ru/j/123", false),
            "https://telemost.yandex.ru/j/123"
        );
        assert_eq!(
            meeting_page("https://telemost.360.yandex.ru/", false),
            "https://telemost.360.yandex.ru/"
        );
    }
}
