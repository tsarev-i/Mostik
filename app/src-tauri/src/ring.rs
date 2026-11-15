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
use mostik_ipc as ipc;
use mostik_ipc::cmd::{self, AnswerArgs, Args, Out};
use mostik_yandex::wire::{self, EndRinging};
use tauri::{AppHandle, Emitter, Manager, UserAttentionType};

/// The ringing call changed: [`Ring`] or `null`.
pub const RING_EVENT: &str = "mostik://ring";

/// Telemost repeats the ringing every 5 s: with no repeat for this long it has stopped ringing.
const STALE: Duration = Duration::from_secs(12);

/// A call ringing for the user. The dialog shows only the caller: the UI gets [`ipc::Ring`].
#[derive(Clone)]
pub struct Ring {
    /// `RingingId`.
    pub id: String,
    pub chat_id: String,
    /// The meeting link.
    pub url: String,
    pub caller: User,
}

impl Ring {
    /// What the incoming call dialog shows.
    fn shown(&self) -> ipc::Ring {
        ipc::Ring {
            caller: self.caller.clone(),
        }
    }
}

#[derive(Default)]
pub struct RingState(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    current: Option<Ring>,
    last_push: Option<Instant>,
    /// Ringings already over: their late repeats must not ring again.
    over: VecDeque<String>,
    /// The last ringing whose repeats stopped coming for a while: a repeat rings it again, and
    /// its end, should it come now, puts it over all the same.
    lapsed: Option<Ring>,
}

impl Inner {
    /// Ends the current ringing and returns it. `over`: it is over for good, its late repeats
    /// must not ring again; else its repeats only stopped coming for a while.
    fn finish(&mut self, over: bool) -> Option<Ring> {
        let ring = self.current.take()?;
        if over {
            self.end(&ring.id);
        } else {
            self.lapsed = Some(ring.clone());
        }
        Some(ring)
    }

    /// Ringing `id` is over for good, whether it rings here now or not.
    fn end(&mut self, id: &str) {
        if self.lapsed.as_ref().is_some_and(|r| r.id == id) {
            self.lapsed = None;
        }
        if !self.over.iter().any(|o| o == id) {
            self.over.push_back(id.to_owned());
            if self.over.len() > 32 {
                self.over.pop_front();
            }
        }
    }

    /// Ends the ringing that `matches` for good: the current one, returned, or the lapsed one.
    fn end_matching(&mut self, matches: impl Fn(&Ring) -> bool) -> Option<Ring> {
        if self.current.as_ref().is_some_and(&matches) {
            return self.finish(true);
        }
        if let Some(lapsed) = self.lapsed.take_if(|r| matches(r)) {
            self.end(&lapsed.id);
        }
        None
    }

    fn rings(&self, id: &str) -> bool {
        self.current.as_ref().is_some_and(|r| r.id == id) && self.last_push.is_some_and(|t| t.elapsed() < STALE)
    }

    /// Ends ringing `id` if its repeats have stopped coming. Returns whether it rings no more:
    /// one whose repeat came meanwhile rings on.
    fn expire(&mut self, id: &str) -> bool {
        if self.current.as_ref().is_none_or(|r| r.id != id) {
            return true;
        }
        if self.rings(id) {
            return false;
        }
        self.finish(false);
        true
    }
}

/// A ringing push: the first one shows the dialog and starts the ringtone, the repeats keep the
/// call ringing.
pub fn incoming(app: &AppHandle, ring: Ring) {
    let state = app.state::<RingState>();
    {
        let Ok(mut inner) = state.0.lock() else { return };
        if inner.over.contains(&ring.id) {
            return;
        }
        inner.last_push = Some(Instant::now());
        if inner.current.as_ref().is_some_and(|r| r.id == ring.id) {
            return;
        }
        // Telemost rings one call at a time: a new one replaces the old.
        inner.finish(true);
        if inner.lapsed.as_ref().is_some_and(|r| r.id == ring.id) {
            inner.lapsed = None;
        }
        inner.current = Some(ring.clone());
        publish(app, &inner);
    }
    attend(app);
    if ring.caller.avatar.is_none() {
        complete_caller(app, ring.clone());
    }
    let (ringer, rid) = (app.clone(), ring.id.clone());
    std::thread::spawn(move || {
        let state = ringer.state::<RingState>();
        loop {
            // Returns once the call stops ringing, with the ringtone or without it: an output
            // the ringtone could not open must not close the dialog.
            crate::sound::ring_while(|| state.0.lock().is_ok_and(|i| i.rings(&rid)));
            // Pushes stopped coming without a `RingingEnded` (the connection dropped, say): the
            // dialog closes, and a repeat that still comes later rings again.
            let Ok(mut inner) = state.0.lock() else { break };
            let ringing = inner.current.is_some();
            if inner.expire(&rid) {
                if ringing && inner.current.is_none() {
                    publish(&ringer, &inner);
                    drop(inner);
                    attend(&ringer);
                }
                break;
            }
        }
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::backend::push_now(&app, wire::notify_ringing(&ring.id)).await {
            eprintln!("NotifyRinging: {e}");
        }
    });
}

/// The ringing `id` is over (`RingingEnded`): the dialog closes, the ringtone stops. A ringing
/// that does not ring here now is over too: its repeats may yet come, late.
pub fn stop(app: &AppHandle, id: &str) {
    if end_if(app, |r| r.id == id).is_none()
        && let Ok(mut inner) = app.state::<RingState>().0.lock()
    {
        inner.end(id);
    }
}

/// The call with link `url` ended.
pub fn call_ended(app: &AppHandle, url: &str) {
    end_if(app, |r| r.url == url);
}

/// Signed out: whatever rings stops, with nothing sent to Telemost.
pub fn forget(app: &AppHandle) {
    end_if(app, |_| true);
}

/// Ends the ringing that `matches` for good, the current or the lapsed one; returns the current one.
fn end_if(app: &AppHandle, matches: impl Fn(&Ring) -> bool) -> Option<Ring> {
    let state = app.state::<RingState>();
    let ring = {
        let mut inner = state.0.lock().ok()?;
        let ring = inner.end_matching(matches)?;
        publish(app, &inner);
        ring
    };
    attend(app);
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

/// Shows the dialog of the current ringing in the main window, or hides it. Called under the
/// lock: the UI gets the changes in the order they were made, so the end of an old ringing does
/// not close the dialog of a new one.
fn publish(app: &AppHandle, inner: &Inner) {
    let _ = app.emit(RING_EVENT, inner.current.as_ref().map(Ring::shown));
}

/// The window is not raised over other programs: out of focus, it asks for attention while a
/// call rings.
fn attend(app: &AppHandle) {
    let ringing = app.state::<RingState>().0.lock().is_ok_and(|i| i.current.is_some());
    if let Some(w) = app.get_webview_window("main") {
        let attention = ringing && !w.is_focused().unwrap_or(false);
        let _ = w.request_user_attention(attention.then_some(UserAttentionType::Critical));
    }
}

/// The push may bring the caller without a name or photo: the dialog gets them from the directory.
fn complete_caller(app: &AppHandle, ring: Ring) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let caller = crate::backend::with_profile(&app, ring.caller).await;
        let state = app.state::<RingState>();
        let Ok(mut inner) = state.0.lock() else { return };
        let Some(cur) = inner.current.as_mut().filter(|r| r.id == ring.id) else {
            return;
        };
        cur.caller = caller;
        publish(&app, &inner);
    });
}

/// The ringing call, for a main window that has just loaded.
#[tauri::command]
pub fn ring_info(app: AppHandle) -> Out<cmd::RingInfo> {
    app.state::<RingState>()
        .0
        .lock()
        .ok()?
        .current
        .as_ref()
        .map(Ring::shown)
}

/// "Accept" or "Decline" in the dialog. Accepting opens the call like "Join" in the chat.
#[tauri::command]
pub async fn answer_ring(app: AppHandle, args: Args<cmd::AnswerRing>) -> Result<Out<cmd::AnswerRing>, String> {
    let AnswerArgs { accept } = args;
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

#[cfg(test)]
mod tests {
    use mostik_core::UserId;

    use super::*;

    fn ring(id: &str) -> Ring {
        Ring {
            id: id.into(),
            chat_id: "a_b".into(),
            url: "https://telemost.360.yandex.ru/j/1".into(),
            caller: User::named(UserId("b".into()), "Boris"),
        }
    }

    /// A ringing whose repeats stopped coming for a while may ring again; one that ended may not.
    #[test]
    fn only_an_ended_ringing_stays_over() {
        let mut inner = Inner {
            current: Some(ring("r1")),
            last_push: Some(Instant::now()),
            ..Inner::default()
        };
        assert!(inner.rings("r1"));
        // A fresh repeat: it rings on.
        assert!(!inner.expire("r1"));
        inner.last_push = Instant::now().checked_sub(STALE);
        assert!(inner.expire("r1"));
        assert!(inner.current.is_none() && inner.over.is_empty());
        // Its call ends while nothing rings: the lapsed ringing is over.
        assert!(inner.end_matching(|r| r.url == ring("r1").url).is_none());
        assert_eq!(inner.over, ["r1"]);
        assert!(inner.lapsed.is_none());
        // `RingingEnded` of a ringing that never rang here, twice.
        inner.end("r2");
        inner.end("r2");
        assert_eq!(inner.over, ["r1", "r2"]);
        inner.current = Some(ring("r3"));
        assert_eq!(
            inner.end_matching(|r| r.id == "r3").map(|r| r.id),
            Some("r3".to_owned())
        );
        assert_eq!(inner.over, ["r1", "r2", "r3"]);
        assert!(inner.finish(true).is_none());
    }
}
