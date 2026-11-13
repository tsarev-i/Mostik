//! The session: startup, backend subscriptions, reconnection, sign-out and the app update.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::{Ctx, Phase};
use crate::bridge::{self, LiveEvent};
use crate::i18n::tr;

use super::chats::{forget_fresh, jump_to_message, load_latest, open_chat};
use super::events::handle_event;
use super::learn_users;
use super::people::refresh_presence;
use super::snapshot::keep_known;
use super::threads::{load_thread, load_threads};

pub fn apply_boot(c: Ctx, boot: Result<bridge::Boot, String>) {
    match boot {
        Ok(bridge::Boot::Ready { snapshot }) => {
            forget_gone_chats(c, &snapshot);
            let first = snapshot.sections.iter().flat_map(|s| s.chats.first()).next().cloned();
            c.snap.set(*snapshot);
            c.phase.set(Phase::Ready);
            c.online.set(true);
            refresh_presence(c);
            load_threads(c);
            if let Some(id) = first {
                open_chat(c, id);
            }
        }
        Ok(bridge::Boot::NeedLogin { reason }) => c.phase.set(Phase::Login {
            reason: reason.or_else(take_sign_out_note),
        }),
        Err(e) => c.phase.set(Phase::Login { reason: Some(e) }),
    }
}

/// Drops what the sidebar keeps in `localStorage` (hidden chats, the custom order) about chats
/// the user is no longer in.
fn forget_gone_chats(c: Ctx, s: &Snapshot) {
    // An empty chat list is more likely a failure than an account without chats.
    if s.chats.is_empty() {
        return;
    }
    let known: std::collections::HashSet<&ChatId> = s.chats.iter().map(|ch| &ch.id).collect();
    if c.hidden.with_untracked(|h| h.keys().any(|id| !known.contains(id))) {
        c.hidden.update(|h| h.retain(|id, _| known.contains(id)));
    }
    if c.chat_order
        .with_untracked(|o| o.values().flatten().any(|id| !known.contains(id)))
    {
        c.chat_order.update(|o| {
            o.retain(|_, ids| {
                ids.retain(|id| known.contains(id));
                !ids.is_empty()
            })
        });
    }
}

/// Keys of `localStorage` that outlive a sign-out: preferences of this computer rather than of
/// the account. The rest of Mostik's keys (drafts, hidden chats, the chat order, the chosen
/// organization, recent emoji…) belong to the account and go with it.
const DEVICE_KEYS: [&str; 3] = ["mostik.settings", "mostik.sidebar_w", "mostik.thread_w"];

/// What went wrong while signing out, kept in `sessionStorage` for the login screen after the
/// reload.
const SIGN_OUT_NOTE: &str = "mostik.sign_out_note";

/// "Sign out": the backend ends the Yandex ID session and clears what it kept; then the
/// account's keys leave `localStorage` and the page loads again, so nothing of the account stays
/// in memory either. The login screen shows any problem on the way.
pub fn sign_out(c: Ctx) {
    c.phase.set(Phase::Loading(tr().signing_out));
    spawn_local(async move {
        let problem = bridge::logout().await.err();
        if let Some(storage) = crate::storage::local() {
            let keys: Vec<String> = (0..storage.length().unwrap_or(0))
                .filter_map(|i| storage.key(i).ok().flatten())
                .filter(|k| k.starts_with("mostik.") && !DEVICE_KEYS.contains(&k.as_str()))
                .collect();
            for key in keys {
                let _ = storage.remove_item(&key);
            }
        }
        if let (Some(problem), Some(session)) = (&problem, window().session_storage().ok().flatten()) {
            let _ = session.set_item(SIGN_OUT_NOTE, problem);
        }
        if window().location().reload().is_err() {
            c.phase.set(Phase::Login { reason: problem });
        }
    });
}

/// The problem the last sign-out left for the login screen, once.
fn take_sign_out_note() -> Option<String> {
    let session = window().session_storage().ok().flatten()?;
    let note = session.get_item(SIGN_OUT_NOTE).ok().flatten()?;
    let _ = session.remove_item(SIGN_OUT_NOTE);
    Some(note)
}

/// Subscriptions to backend events.
pub fn subscribe(c: Ctx) {
    spawn_local(async move {
        bridge::listen(bridge::EVENT, move |ev: LiveEvent| handle_event(c, ev)).await;
        bridge::listen(bridge::OPEN_EVENT, move |t: bridge::OpenTarget| {
            jump_to_message(c, t.chat_id, t.ts, t.thread_parent);
        })
        .await;
        bridge::listen(bridge::DRAG_EVENT, move |at: Option<(f64, f64)>| {
            let zone = at.and_then(|(x, y)| crate::compose::drop_zone_physical(x, y));
            crate::compose::highlight_drop(zone == Some(crate::compose::DropZone::Chat));
        })
        .await;
        bridge::listen(bridge::FILES_EVENT, move |d: bridge::Dropped| {
            crate::compose::highlight_drop(false);
            match crate::compose::drop_zone_physical(d.x, d.y) {
                Some(crate::compose::DropZone::Chat) => super::attach_files(c, c.current.get_untracked(), d.files),
                Some(crate::compose::DropZone::Thread) => c.notice.set(Some(tr().no_files_in_thread.into())),
                None => {}
            }
        })
        .await;
        bridge::listen(bridge::CALL_EVENT, move |call: Option<bridge::CallInfo>| {
            if call.is_some() {
                c.notice.set(None);
            }
            c.call.set(call);
        })
        .await;
        if let Ok(call) = bridge::current_call().await {
            c.call.set(call);
        }
        bridge::listen(bridge::RING_EVENT, move |ring: Option<bridge::Ring>| c.ring.set(ring)).await;
        // A call may have started ringing before the subscription.
        if let Ok(ring) = bridge::ring_info().await {
            c.ring.set(ring);
        }
        bridge::listen(bridge::SIGNED_OUT_EVENT, move |reason: String| {
            c.phase.set(Phase::Login { reason: Some(reason) });
        })
        .await;
        bridge::listen(bridge::SNAPSHOT_EVENT, move |snapshot: Snapshot| {
            reconnected(c, snapshot)
        })
        .await;
        bridge::listen(
            bridge::EMOJI_EVENT,
            move |sets: Vec<mostik_core::custom_emoji::EmojiSet>| c.custom.set(sets),
        )
        .await;
        if let Ok(sets) = bridge::custom_emoji().await {
            c.custom.set(sets);
        }
        bridge::listen(bridge::UPDATE_EVENT, move |status: bridge::UpdateStatus| {
            c.update.set(status)
        })
        .await;
        // The first check may have finished before the subscription.
        if let Ok(status) = bridge::update_status().await {
            c.update.set(status);
        }
    });
}

/// The fresh snapshot after a reconnection. The chat list, counters and presence come from it,
/// but the messages already shown stay (the snapshot has only the last one of each chat), so the
/// open chat does not blank out; it then gets its latest page in place, keeping an edit in
/// progress. An open thread stays and gets the replies missed while offline.
fn reconnected(c: Ctx, mut fresh: Snapshot) {
    forget_fresh();
    // Directory profiles may have changed meanwhile: they load again when opened, all but the
    // one on screen.
    let open = c.profile.get_untracked();
    c.profiles.update(|p| p.retain(|id, _| Some(id) == open.as_ref()));
    c.snap.update(|s| {
        let old = std::mem::take(&mut s.messages);
        // The snapshot has no presence (everyone is offline in it): known people keep theirs
        // until the poll below, otherwise every dot turns gray after each reconnection.
        for u in &mut fresh.users {
            if let Some(known) = s.users.iter().find(|x| x.id == u.id) {
                u.presence = known.presence;
                u.availability = known.availability;
                u.status = known.status.clone();
            }
        }
        learn_users(&mut fresh, std::mem::take(&mut s.users));
        let mut messages: Vec<Message> = fresh.messages.into_iter().map(|m| keep_known(m, &old)).collect();
        for m in old {
            if !messages.iter().any(|x| x.chat_id == m.chat_id && x.ts == m.ts) {
                messages.push(m);
            }
        }
        fresh.messages = messages;
        *s = fresh;
    });
    refresh_presence(c);
    load_threads(c);
    let current = c.current.get_untracked();
    if !current.0.is_empty() {
        if let Some(parent) = c.thread.get_untracked() {
            load_thread(c, current.clone(), parent);
        }
        spawn_local(async move { load_latest(c, &current).await });
    }
}

/// Installs the available update; the app restarts on success, so only an error comes back.
pub fn install_update(c: Ctx) {
    spawn_local(async move {
        if let Err(e) = bridge::install_update().await {
            c.error.set(Some(e));
        }
    });
}
