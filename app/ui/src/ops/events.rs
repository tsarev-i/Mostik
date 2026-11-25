//! Backend events: what a push from Telemost changes in the UI state.

use leptos::prelude::*;
use mostik_core::*;

use crate::app::Ctx;
use crate::bridge::LiveEvent;

use super::chats::{drop_chat, forget_fresh, peer_seen};
use super::learn_users;
use super::messages::set_pinned;
use super::people::{LAST_HEARTBEAT, apply_presence};
use super::sidebar::apply_starred;
use super::snapshot::{remove, sort_sections, upsert, with_call_length};
use super::threads::merge_threads;

thread_local! {
    /// Role changes of chats the UI does not have yet: the snapshot is on its way (at the start,
    /// after a reconnection), or the chat is being fetched. They apply when the chat comes, if
    /// still newer than its role there.
    static HELD_ROLES: std::cell::RefCell<std::collections::HashMap<ChatId, MyRole>> = Default::default();
}

/// How many chats [`HELD_ROLES`] holds role changes for; then it starts afresh.
const HELD_ROLES_MAX: usize = 200;

fn hold_role(chat_id: ChatId, role: MyRole) {
    HELD_ROLES.with_borrow_mut(|held| {
        if held.len() >= HELD_ROLES_MAX {
            held.clear();
        }
        let newer = held.get(&chat_id).is_none_or(|old| old.version <= role.version);
        if newer {
            held.insert(chat_id, role);
        }
    });
}

/// Applies to `chats` the role changes held for them (see [`HELD_ROLES`]).
pub(super) fn apply_held_roles(chats: &mut [Chat]) {
    HELD_ROLES.with_borrow_mut(|held| {
        if held.is_empty() {
            return;
        }
        for ch in chats.iter_mut() {
            if let Some(role) = held.remove(&ch.id)
                && ch.role.is_none_or(|r| r.version < role.version)
            {
                ch.role = Some(role);
            }
        }
    });
}

pub fn handle_event(c: Ctx, ev: LiveEvent) {
    match ev {
        LiveEvent::Upsert {
            message,
            author,
            mentioned,
            ..
        } => {
            if !mentioned.is_empty() {
                c.snap.update(|s| learn_users(s, mentioned));
            }
            let message = c.meetings.with_untracked(|m| with_call_length(*message, m));
            upsert(c, message, author)
        }
        LiveEvent::Deleted {
            chat_id,
            ts,
            thread_parent,
        } => remove(c, &chat_id, ts, thread_parent),
        LiveEvent::Reactions {
            chat_id,
            ts,
            reactions,
            users,
            ..
        } => {
            let mut loaded = false;
            c.snap.update(|s| {
                learn_users(s, users);
                if let Some(m) = s.messages.iter_mut().find(|m| m.chat_id == chat_id && m.ts == ts) {
                    m.reactions = reactions;
                    loaded = true;
                }
            });
            // A message outside the feed: a copy asked for alone (a pinned one) is asked for again.
            if !loaded {
                c.older_changed.set(Some((chat_id, ts)));
            }
        }
        LiveEvent::ThreadSeen { chat_id, parent } => c.threads.update(|list| {
            if let Some(t) = list
                .iter_mut()
                .find(|t| t.parent.chat_id == chat_id && t.parent.ts == parent)
            {
                t.read();
            }
        }),
        LiveEvent::Seen { chat_id } => c.snap.update(|s| s.read_chat(&chat_id)),
        LiveEvent::PeerSeen {
            chat_id,
            user,
            ts,
            reads,
        } => peer_seen(c, chat_id, user, ts, reads),
        LiveEvent::Typing { chat_id, user } => {
            let now = js_sys::Date::now();
            c.typing.update(|t| {
                // Lines that have run out go too: the list keeps only the people typing now.
                t.retain(|(ch, u, until)| *until > now && !(ch == &chat_id && u == &user));
                t.push((chat_id, user, now + 5000.0));
            });
        }
        LiveEvent::Online { user } => {
            LAST_HEARTBEAT.with_borrow_mut(|m| m.insert(user.clone(), js_sys::Date::now()));
            c.snap.update(|s| {
                if let Some(u) = s.users.iter_mut().find(|u| u.id == user) {
                    u.presence = Presence::Online;
                }
            })
        }
        LiveEvent::Status(p) => apply_presence(c, vec![p]),
        LiveEvent::Pinned { chat_id, ts } => set_pinned(c, &chat_id, ts),
        LiveEvent::ChatAdded {
            mut chat,
            section,
            users,
        } => c.snap.update(|s| {
            apply_held_roles(std::slice::from_mut(&mut chat));
            learn_users(s, users);
            if s.chat(&chat.id).is_some() {
                return;
            }
            if let Some(sec) = s.sections.iter_mut().find(|sec| sec.id == section)
                && !sec.chats.contains(&chat.id)
            {
                sec.chats.push(chat.id.clone());
            }
            s.chats.push(chat);
            sort_sections(s);
        }),
        LiveEvent::Connection { online } => {
            if !online {
                forget_fresh();
                // Telemost reports a call when it changes: one going on may change unseen now.
                c.meetings.update(|m| m.retain(|_, m| m.ended));
            }
            c.online.set(online)
        }
        LiveEvent::Users { users } => c.snap.update(|s| learn_users(s, users)),
        LiveEvent::Threads { threads, users } => {
            c.snap.update(|s| learn_users(s, users));
            merge_threads(c, threads);
        }
        // The backend normally swallows them: it sends `ChatAdded` and `Threads` instead.
        LiveEvent::Joined { .. } | LiveEvent::ThreadJoined { .. } => {}
        LiveEvent::RoleChanged { chat_id, role } => {
            c.snap.update(|s| match s.chats.iter_mut().find(|ch| ch.id == chat_id) {
                Some(ch) => {
                    if ch.role.is_none_or(|old| old.version <= role.version) {
                        ch.role = Some(role);
                    }
                }
                None => hold_role(chat_id, role),
            })
        }
        LiveEvent::ChatRemoved { chat_id } => drop_chat(c, &chat_id),
        LiveEvent::ChatChanged {
            chat_id,
            name,
            description,
            member_count,
        } => c.snap.update(|s| {
            let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == chat_id) else {
                return;
            };
            // A direct chat is named after the person, whatever its chat name says.
            if let Some(name) = name
                && matches!(ch.kind, ChatKind::Group { .. } | ChatKind::Broadcast)
            {
                ch.name = name;
            }
            if let Some(d) = description {
                ch.description = Some(d).filter(|d| !d.is_empty());
            }
            if let Some(n) = member_count {
                ch.member_count = n;
            }
        }),
        LiveEvent::Folders { folders } => {
            if c.snap.with_untracked(|s| s.folders != folders) {
                c.snap.update(|s| s.folders = folders);
            }
        }
        LiveEvent::Starred { chats } => {
            let same = c.snap.with_untracked(|s| {
                let known: Vec<&ChatId> = chats.iter().filter(|id| s.chat(id).is_some()).collect();
                let now: Vec<&ChatId> = s
                    .sections
                    .iter()
                    .find(|sec| sec.id == "starred")
                    .map(|sec| sec.chats.iter().collect())
                    .unwrap_or_default();
                known == now
            });
            if !same {
                c.snap.update(|s| apply_starred(s, chats));
            }
        }
        LiveEvent::Meeting(m) => c.meetings.update(|list| {
            list.insert(m.chat_id.clone(), m);
        }),
        LiveEvent::Paused { pause } => {
            if c.snap.with_untracked(|s| s.paused != pause) {
                c.snap.update(|s| s.paused = pause);
            }
        }
        LiveEvent::Mutings { chats } => c.snap.update(|s| {
            for ch in &mut s.chats {
                let level = chats
                    .iter()
                    .find(|(id, _)| id == &ch.id)
                    .map_or(NotifyLevel::All, |(_, level)| *level);
                ch.set_notify_level(level);
            }
        }),
    }
}
