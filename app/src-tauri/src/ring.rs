//! Incoming direct calls, like Slack's call popup: a small window on top of everything with the
//! caller and "Decline" / "Accept", and a ringtone in a loop while the call rings.
//!
//! Telemost rings with a `MeetingCallingMessage.IncomingCall` push repeated every 5 s and stops
//! with `RingingEnded` (the caller hung up, someone answered, maybe on another device, or nobody
//! did in time). Mostik answers like the web client: `NotifyRinging` when the popup shows,
//! `EndRinging` with "accepted" or "declined" on a button.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use mostik_core::User;
use mostik_yandex::wire::{self, EndRinging};
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// Label prefix of popup windows; each ringing gets a window of its own.
pub const LABEL_PREFIX: &str = "ring-";

/// Telemost repeats the ringing every 5 s: with no repeat for this long it has stopped ringing.
const STALE: Duration = Duration::from_secs(12);

/// A call ringing for the user.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ring {
    /// `RingingId`.
    #[serde(skip)]
    pub id: String,
    pub chat_id: String,
    /// The meeting link.
    pub url: String,
    pub caller: User,
}

#[derive(Default)]
pub struct RingState(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    current: Option<Ring>,
    /// The popup of the current ringing.
    window: Option<String>,
    last_push: Option<Instant>,
    /// Ringings already over: their late repeats must not ring again.
    over: VecDeque<String>,
}

impl Inner {
    /// Ends the current ringing; returns it and its popup.
    fn finish(&mut self) -> Option<(Ring, Option<String>)> {
        let ring = self.current.take()?;
        self.over.push_back(ring.id.clone());
        if self.over.len() > 32 {
            self.over.pop_front();
        }
        Some((ring, self.window.take()))
    }

    fn rings(&self, id: &str) -> bool {
        self.current.as_ref().is_some_and(|r| r.id == id) && self.last_push.is_some_and(|t| t.elapsed() < STALE)
    }
}

static WINDOWS: AtomicU64 = AtomicU64::new(0);

/// A ringing push: the first one opens the popup and starts the ringtone, the repeats keep the
/// call ringing.
pub fn incoming(app: &AppHandle, ring: Ring) {
    let state = app.state::<RingState>();
    let Ok(mut inner) = state.0.lock() else { return };
    if inner.over.contains(&ring.id) {
        return;
    }
    inner.last_push = Some(Instant::now());
    if inner.current.as_ref().is_some_and(|r| r.id == ring.id) {
        return;
    }
    // Telemost rings one call at a time: a new one replaces the old.
    let old = inner.finish().and_then(|(_, w)| w);
    let id = ring.id.clone();
    let label = format!("{LABEL_PREFIX}{}", WINDOWS.fetch_add(1, Ordering::SeqCst));
    inner.current = Some(ring);
    inner.window = Some(label.clone());
    drop(inner);
    if let Some(old) = old {
        close(app, &old);
    }
    if let Err(e) = open_popup(app, &label) {
        eprintln!("call popup not shown: {e}");
    }
    let (ringer, rid) = (app.clone(), id.clone());
    std::thread::spawn(move || {
        let state = ringer.state::<RingState>();
        crate::sound::ring_while(|| state.0.lock().is_ok_and(|i| i.rings(&rid)));
        // Pushes stopped coming without a `RingingEnded` (the connection dropped, say).
        let stale = state
            .0
            .lock()
            .is_ok_and(|i| i.current.as_ref().is_some_and(|r| r.id == rid));
        if stale {
            stop(&ringer, &rid);
        }
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::backend::push_now(&app, wire::notify_ringing(&id)).await {
            eprintln!("NotifyRinging: {e}");
        }
    });
}

/// The ringing `id` is over (`RingingEnded`): the popup closes, the ringtone stops.
pub fn stop(app: &AppHandle, id: &str) {
    end_if(app, |r| r.id == id);
}

/// The call with link `url` ended.
pub fn call_ended(app: &AppHandle, url: &str) {
    end_if(app, |r| r.url == url);
}

fn end_if(app: &AppHandle, matches: impl Fn(&Ring) -> bool) -> Option<Ring> {
    let state = app.state::<RingState>();
    let (ring, window) = {
        let mut inner = state.0.lock().ok()?;
        if !inner.current.as_ref().is_some_and(&matches) {
            return None;
        }
        inner.finish()?
    };
    if let Some(w) = window {
        close(app, &w);
    }
    Some(ring)
}

/// The user joins the call with link `url` (from the popup or "Join" in the chat): if that call
/// is ringing, it stops ringing on all the user's devices.
pub fn accept(app: &AppHandle, url: &str) {
    if let Some(ring) = end_if(app, |r| r.url == url) {
        send_end(app, ring.id, EndRinging::Accepted);
    }
}

fn send_end(app: &AppHandle, id: String, reason: EndRinging) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::backend::push_now(&app, wire::end_ringing(&id, reason)).await {
            eprintln!("EndRinging: {e}");
        }
    });
}

fn open_popup(app: &AppHandle, label: &str) -> tauri::Result<()> {
    let title = crate::backend::text(app, "Входящий звонок", "Incoming call");
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html#ring".into()))
        .title(title)
        .inner_size(360.0, 196.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .center()
        .focused(true)
        .build()?;
    crate::webview::harden(&window);
    Ok(())
}

fn close(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.destroy();
    }
}

/// The ringing call, for the popup that has just loaded. The caller's photo is looked up if the
/// push did not bring it.
#[tauri::command]
pub async fn ring_info(app: AppHandle) -> Option<Ring> {
    let mut ring = app.state::<RingState>().0.lock().ok()?.current.clone()?;
    if ring.caller.avatar.is_none() {
        ring.caller = crate::backend::with_profile(&app, ring.caller).await;
        let state = app.state::<RingState>();
        if let Ok(mut inner) = state.0.lock()
            && let Some(cur) = inner.current.as_mut().filter(|r| r.id == ring.id)
        {
            cur.caller = ring.caller.clone();
        }
    }
    Some(ring)
}

/// "Accept" or "Decline" in the popup. Accepting opens the call like "Join" in the chat.
#[tauri::command]
pub async fn answer_ring(app: AppHandle, accept: bool) -> Result<(), String> {
    let ring = {
        let state = app.state::<RingState>();
        let inner = state.0.lock().map_err(|e| e.to_string())?;
        inner.current.clone()
    };
    let Some(ring) = ring else {
        // The caller hung up a moment ago: the popup is about to close anyway.
        return Ok(());
    };
    if !accept {
        end_if(&app, |r| r.id == ring.id);
        send_end(&app, ring.id, EndRinging::Declined);
        return Ok(());
    }
    // `open_call` stops the ringing as accepted.
    crate::backend::open_call(&app, ring.chat_id, ring.caller.display_name, Some(ring.url)).await
}
