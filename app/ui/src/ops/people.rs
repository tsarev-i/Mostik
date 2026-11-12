//! People: profiles, direct messages, presence and the user's own status.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::{Ctx, Phase};
use crate::bridge;
use crate::i18n::tr;

use super::chats::open_chat;
use super::{failed, now_mcs};

/// Opens the "Profile" panel (in place of the thread, as in Slack) and loads data from the directory.
pub fn open_profile(c: Ctx, user: UserId) {
    c.thread.set(None);
    c.profile.set(Some(user.clone()));
    if !c.live || c.profiles.with_untracked(|p| p.contains_key(&user)) {
        return;
    }
    spawn_local(async move {
        match bridge::user_profile(&user.0).await {
            Ok(p) => c.profiles.update(|m| {
                m.insert(user, p);
            }),
            Err(e) => failed(c, tr().err_profile, e),
        }
    });
}

/// "Message" in a profile: opens the direct chat, creating it if needed.
pub fn message_user(c: Ctx, user: UserId) {
    let existing = c.snap.with_untracked(|s| {
        s.chats
            .iter()
            .find(|ch| match &ch.kind {
                ChatKind::Direct { peer } => peer == &user,
                ChatKind::Notes => user == s.me,
                _ => false,
            })
            .map(|ch| ch.id.clone())
    });
    c.profile.set(None);
    if let Some(id) = existing {
        return open_chat(c, id);
    }
    if !c.live {
        return;
    }
    spawn_local(async move {
        match bridge::open_dm(&user.0).await {
            Ok(chat) => {
                let id = chat.id.clone();
                c.snap.update(|s| {
                    if s.chat(&id).is_none() {
                        s.chats.push(chat);
                        if let Some(sec) = s.sections.iter_mut().find(|sec| sec.id == "dms") {
                            sec.chats.push(id.clone());
                        }
                    }
                });
                open_chat(c, id);
            }
            Err(e) => failed(c, tr().err_open_dm, e),
        }
    });
}

/// How long after a pushed `Heartbeat` a person stays online, whatever a poll says
/// (the same window as `ONLINE_WINDOW_MCS` in `mostik-yandex`).
const ONLINE_WINDOW_MS: f64 = 10.0 * 60.0 * 1000.0;

thread_local! {
    /// When the last `Heartbeat` push of each person arrived (local time, ms).
    pub(super) static LAST_HEARTBEAT: std::cell::RefCell<std::collections::HashMap<UserId, f64>> =
        Default::default();
}

/// Applies presence and statuses to the people of the snapshot. We always count ourselves as online:
/// while Mostik runs, it sends `Heartbeat`. A poll does not turn gray someone whose `Heartbeat`
/// push came within the online window: the poll may lag behind the push.
pub fn apply_presence(c: Ctx, list: Vec<PresenceInfo>) {
    let now = js_sys::Date::now();
    let fresh = |user: &UserId| LAST_HEARTBEAT.with_borrow(|m| m.get(user).is_some_and(|t| now - t < ONLINE_WINDOW_MS));
    c.snap.update(|s| {
        let me = s.me.clone();
        for p in list {
            if let Some(u) = s.users.iter_mut().find(|u| u.id == p.user) {
                if let Some(online) = p.online {
                    u.presence = if online || fresh(&u.id) {
                        Presence::Online
                    } else {
                        Presence::Offline
                    };
                }
                u.availability = p.availability;
                u.status = p.status.clone();
            }
        }
        if let Some(u) = s.users.iter_mut().find(|u| u.id == me) {
            u.presence = Presence::Online;
        }
    });
}

/// Turns gray the people whose last pushed `Heartbeat` is older than the online window: going
/// offline is not pushed. People known online only from a poll wait for the next poll.
pub fn expire_presence(c: Ctx) {
    let now = js_sys::Date::now();
    let stale: Vec<UserId> = LAST_HEARTBEAT.with_borrow(|m| {
        m.iter()
            .filter(|(_, t)| now - **t >= ONLINE_WINDOW_MS)
            .map(|(u, _)| u.clone())
            .collect()
    });
    if stale.is_empty() {
        return;
    }
    LAST_HEARTBEAT.with_borrow_mut(|m| m.retain(|u, _| !stale.contains(u)));
    let gone = c.snap.with_untracked(|s| {
        s.users
            .iter()
            .any(|u| u.id != s.me && u.presence == Presence::Online && stale.contains(&u.id))
    });
    if gone {
        c.snap.update(|s| {
            let me = s.me.clone();
            for u in s.users.iter_mut().filter(|u| u.id != me && stale.contains(&u.id)) {
                u.presence = Presence::Offline;
            }
        });
    }
}

/// Requests the presence of everyone in the snapshot (direct chat peers first).
pub fn refresh_presence(c: Ctx) {
    if !c.live {
        return;
    }
    let guids: Vec<String> = c.snap.with_untracked(|s| {
        let mut ids: Vec<String> = s
            .chats
            .iter()
            .filter_map(|ch| match &ch.kind {
                ChatKind::Direct { peer } => Some(peer.0.clone()),
                _ => None,
            })
            .collect();
        ids.push(s.me.0.clone());
        for u in &s.users {
            if !ids.contains(&u.id.0) {
                ids.push(u.id.0.clone());
            }
        }
        ids
    });
    spawn_local(async move {
        if let Ok(list) = bridge::presence(guids).await {
            apply_presence(c, list);
        }
    });
}

/// Own name and photo from the directory, so a new profile picture shows up without a restart.
/// Runs when the window gets focus (the picture is changed in the browser or on another device),
/// at most once in 10 minutes.
pub fn refresh_me(c: Ctx) {
    thread_local! {
        static LAST: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    }
    let now = js_sys::Date::now();
    if !c.live || c.phase.get_untracked() != Phase::Ready || now - LAST.get() < 10.0 * 60_000.0 {
        return;
    }
    LAST.set(now);
    let me = c.snap.with_untracked(|s| s.me.clone());
    spawn_local(async move {
        let Ok(p) = bridge::user_profile(&me.0).await else {
            return;
        };
        c.snap.update(|s| {
            if let Some(u) = s.users.iter_mut().find(|u| u.id == p.id) {
                if p.avatar.is_some() && u.avatar != p.avatar {
                    u.avatar = p.avatar.clone();
                }
                if !p.display_name.is_empty() && u.display_name != p.display_name {
                    u.display_name = p.display_name.clone();
                }
            }
        });
        c.profiles.update(|m| {
            m.insert(p.id.clone(), p);
        });
    });
}

/// Own availability and status, for `duration` seconds or until changed. "Do not disturb" also
/// pauses notifications on every device, as in Telemost.
pub fn set_status(c: Ctx, availability: Availability, status: Option<CustomStatus>, duration: Option<u64>) {
    let me = c.snap.with_untracked(|s| s.me.clone());
    let (emoji, text) = status
        .clone()
        .map_or((String::new(), String::new()), |s| (s.emoji, s.text));
    let apply = move |c: Ctx| {
        apply_presence(
            c,
            vec![PresenceInfo {
                user: me.clone(),
                online: Some(true),
                availability,
                status: status.clone(),
            }],
        );
        // What Telemost will say in the `status` bucket a moment later.
        let pause = (availability == Availability::DoNotDisturb).then(|| Pause {
            until: duration.map(|d| now_mcs() + d * 1_000_000),
        });
        c.snap.update(|s| s.paused = pause);
    };
    if !c.live {
        return apply(c);
    }
    spawn_local(async move {
        match bridge::set_status(availability.code(), &emoji, &text, duration).await {
            Ok(()) => apply(c),
            Err(e) => failed(c, tr().err_status, e),
        }
    });
}
