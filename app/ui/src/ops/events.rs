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

pub fn handle_event(c: Ctx, ev: LiveEvent) {
    match ev {
        LiveEvent::Upsert {
            message,
            author,
            mentioned,
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
        } => c.snap.update(|s| {
            learn_users(s, users);
            if let Some(m) = s.messages.iter_mut().find(|m| m.chat_id == chat_id && m.ts == ts) {
                m.reactions = reactions;
            }
        }),
        LiveEvent::ThreadSeen { chat_id, parent } => c.threads.update(|list| {
            if let Some(t) = list
                .iter_mut()
                .find(|t| t.parent.chat_id == chat_id && t.parent.ts == parent)
            {
                t.unread = 0;
            }
        }),
        LiveEvent::Seen { chat_id } => c.snap.update(|s| {
            if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == chat_id) {
                ch.unread = 0;
                ch.mentions = 0;
            }
        }),
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
        LiveEvent::ChatAdded { chat, section, users } => c.snap.update(|s| {
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
