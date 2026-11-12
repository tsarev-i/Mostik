//! Incoming direct calls, like Slack's: a dialog in the main window with the caller and
//! "Decline" / "Accept", and a ringtone in a loop while the call rings. The dialog stays inside
//! the app, like the preferences, instead of popping up over other programs: a window out of
//! focus asks for attention (a blinking taskbar button) instead.
//!
//! Telemost rings with a `MeetingCallingMessage.IncomingCall` push repeated every 5 s and stops
//! with `RingingEnded` (the caller hung up, someone answered, maybe on another device, or nobody
//! did in time). Mostik answers like the web client: `NotifyRinging` when the dialog shows,
//! `EndRinging` with "accepted" or "declined" on a button.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mostik_core::User;
use mostik_yandex::wire::{self, EndRinging};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, UserAttentionType};

/// The ringing call changed: [`Ring`] or `null`.
pub const RING_EVENT: &str = "mostik://ring";

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
    last_push: Option<Instant>,
    /// Ringings already over: their late repeats must not ring again.
    over: VecDeque<String>,
}

impl Inner {
    /// Ends the current ringing and returns it.
    fn finish(&mut self) -> Option<Ring> {
        let ring = self.current.take()?;
        self.over.push_back(ring.id.clone());
        if self.over.len() > 32 {
            self.over.pop_front();
        }
        Some(ring)
    }

    fn rings(&self, id: &str) -> bool {
        self.current.as_ref().is_some_and(|r| r.id == id) && self.last_push.is_some_and(|t| t.elapsed() < STALE)
    }
}

/// A ringing push: the first one shows the dialog and starts the ringtone, the repeats keep the
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
    inner.finish();
    let id = ring.id.clone();
    inner.current = Some(ring.clone());
    drop(inner);
    show(app, Some(&ring));
    if ring.caller.avatar.is_none() {
        complete_caller(app, ring);
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

/// The ringing `id` is over (`RingingEnded`): the dialog closes, the ringtone stops.
pub fn stop(app: &AppHandle, id: &str) {
    end_if(app, |r| r.id == id);
}

/// The call with link `url` ended.
pub fn call_ended(app: &AppHandle, url: &str) {
    end_if(app, |r| r.url == url);
}

/// Signed out: whatever rings stops, with nothing sent to Telemost.
pub fn forget(app: &AppHandle) {
    end_if(app, |_| true);
}

fn end_if(app: &AppHandle, matches: impl Fn(&Ring) -> bool) -> Option<Ring> {
    let state = app.state::<RingState>();
    let ring = {
        let mut inner = state.0.lock().ok()?;
        if !inner.current.as_ref().is_some_and(&matches) {
            return None;
        }
        inner.finish()?
    };
    show(app, None);
    Some(ring)
}

/// The user joins the call with link `url` (from the dialog or "Join" in the chat): if that call
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

/// Shows the dialog of `ring` in the main window, or hides it. The window is not raised over other
/// programs: out of focus, it asks for attention while the call rings.
fn show(app: &AppHandle, ring: Option<&Ring>) {
    let _ = app.emit(RING_EVENT, ring);
    if let Some(w) = app.get_webview_window("main") {
        let attention = ring.is_some() && !w.is_focused().unwrap_or(false);
        let _ = w.request_user_attention(attention.then_some(UserAttentionType::Critical));
    }
}

/// The push may bring the caller without a name or photo: the dialog gets them from the directory.
fn complete_caller(app: &AppHandle, ring: Ring) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let caller = crate::backend::with_profile(&app, ring.caller).await;
        let state = app.state::<RingState>();
        let updated = {
            let Ok(mut inner) = state.0.lock() else { return };
            let Some(cur) = inner.current.as_mut().filter(|r| r.id == ring.id) else {
                return;
            };
            cur.caller = caller;
            cur.clone()
        };
        let _ = app.emit(RING_EVENT, Some(&updated));
    });
}

/// The ringing call, for a main window that has just loaded.
#[tauri::command]
pub fn ring_info(app: AppHandle) -> Option<Ring> {
    app.state::<RingState>().0.lock().ok()?.current.clone()
}

/// "Accept" or "Decline" in the dialog. Accepting opens the call like "Join" in the chat.
#[tauri::command]
pub async fn answer_ring(app: AppHandle, accept: bool) -> Result<(), String> {
    let ring = {
        let state = app.state::<RingState>();
        let inner = state.0.lock().map_err(|e| e.to_string())?;
        inner.current.clone()
    };
    let Some(ring) = ring else {
        // The caller hung up a moment ago: the dialog is about to close anyway.
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
