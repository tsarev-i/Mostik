//! Messages in the snapshot: adding, replacing and removing them with the thread and unread
//! counters, the section order, and what a message sent again keeps from the known one.

use leptos::prelude::*;
use mostik_core::*;

use crate::app::Ctx;

use super::chats::{loaded_from, mark_read};
use super::learn_users;
use super::threads::track_thread;

/// Inserts or replaces a message; keeps the thread and unread counters up to date.
pub fn upsert(c: Ctx, m: Message, author: Option<User>) {
    let current = c.current.get_untracked();
    let open_thread = c.thread.get_untracked();
    let viewing =
        current == m.chat_id && document_visible() && c.hold_unread.with_untracked(|h| h.as_ref() != Some(&m.chat_id));
    let start = loaded_from(&m);
    let mut shown = true;
    let mut added = false;
    c.snap.update(|s| {
        // A push carries the author's current name and photo: they replace a stand-in like
        // "Unknown user" and an old picture.
        if let Some(a) = author {
            if let Some(known) = s.users.iter_mut().find(|u| u.id == a.id)
                && a.avatar.is_some()
            {
                known.avatar = a.avatar.clone();
            }
            learn_users(s, vec![a]);
        }
        if let Some(old) = s.messages.iter_mut().find(|x| x.chat_id == m.chat_id && x.ts == m.ts) {
            // An edit push may come without reactions or a sequence number: keep the known ones.
            // Reaction changes arrive as separate `Reactions` events.
            let thread = match (m.thread.clone(), &old.thread) {
                (Some(mut t), Some(known)) => {
                    t.keep_known(known);
                    Some(t)
                }
                (t, known) => t.or_else(|| known.clone()),
            };
            let reactions = if m.reactions.is_empty() {
                std::mem::take(&mut old.reactions)
            } else {
                m.reactions.clone()
            };
            let seq_no = if m.seq_no == 0 { old.seq_no } else { m.seq_no };
            let mut fresh = Message {
                thread,
                reactions,
                seq_no,
                ..m.clone()
            };
            fresh.keep_reads(old);
            *old = fresh;
            return;
        }
        added = add_new(s, &m, viewing, start);
        shown = added;
    });
    if !shown {
        return;
    }
    if let Some(parent) = m.thread_parent
        && added
        && !m.edited
    {
        track_thread(c, &m, parent);
    }
    let viewing_reply = m.thread_parent.is_some() && open_thread == m.thread_parent;
    let read_on_open = c.settings.with_untracked(|s| s.mark_read_on_open);
    if c.live && read_on_open && reading(c, &m.chat_id) && (m.thread_parent.is_none() || viewing_reply) {
        mark_read(c, m.chat_id);
    }
}

/// Adds message `m`, not loaded so far, and keeps the counters: an unread message from someone
/// else in a chat the user is not `viewing` adds to the chat's unread and mentions, a reply to
/// its thread's replies. Telemost pushes an edit as an ordinary message, so an edit counts
/// nowhere, and a message older than the loaded part of its chat or thread (which starts at
/// `start` when known) stays out of the feed. Returns whether the message was added.
pub(super) fn add_new(s: &mut Snapshot, m: &Message, viewing: bool, start: Option<Timestamp>) -> bool {
    let oldest = s
        .messages
        .iter()
        .filter(|x| x.chat_id == m.chat_id && x.thread_parent == m.thread_parent)
        .map(|x| x.ts)
        .min();
    if start.or(oldest).is_some_and(|from| m.ts < from) {
        return false;
    }
    let mine = m.author == s.me;
    // Only an unread feed message from someone else can add a mention. A ringing call is not
    // a message: Telemost counts only the note that the call ended.
    let counts = !mine && !viewing && !m.edited && m.thread_parent.is_none() && !m.is_ringing();
    let mentioned = counts && s.mentions_me(&m.text);
    if let Some(parent) = m.thread_parent {
        if !m.edited
            && let Some(p) = s.messages.iter_mut().find(|x| x.chat_id == m.chat_id && x.ts == parent)
        {
            let t = p.thread.get_or_insert(ThreadSummary {
                reply_count: 0,
                last_reply_ts: m.ts,
                participants: vec![],
            });
            t.reply_count += 1;
            t.last_reply_ts = t.last_reply_ts.max(m.ts);
            if !t.participants.contains(&m.author) {
                t.participants.push(m.author.clone());
            }
        }
    } else if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == m.chat_id) {
        ch.last_ts = ch.last_ts.max(m.ts);
        if counts {
            ch.unread += 1;
            if mentioned {
                ch.mentions += 1;
            }
        }
    }
    s.messages.push(m.clone());
    sort_sections(s);
    true
}

/// Whether chat `id` is in front of the user, so what comes in it is read: it is open and not
/// covered by a page of the Home view, the user has not marked it unread while in it, and the
/// window is not hidden.
pub(super) fn reading(c: Ctx, id: &ChatId) -> bool {
    c.current.with_untracked(|cur| cur == id)
        && !c.view.get_untracked().is_home_page()
        && c.hold_unread.with_untracked(|h| h.as_ref() != Some(id))
        && document_visible()
}

pub(super) fn document_visible() -> bool {
    !document().hidden()
}

/// Chats in the sections ordered by recency, as in Telemost.
pub(super) fn sort_sections(s: &mut Snapshot) {
    // Key: the chat with oneself always comes first, the rest by recency.
    let key: std::collections::HashMap<ChatId, (bool, Timestamp)> = s
        .chats
        .iter()
        .map(|c| (c.id.clone(), (c.kind == ChatKind::Notes, c.last_ts)))
        .collect();
    for sec in &mut s.sections {
        if sec.id != "starred" {
            sec.chats.sort_by(|a, b| key.get(b).cmp(&key.get(a)));
        }
    }
}

/// A channel the user has just joined or created goes on top of "Channels". The push about it
/// may have brought it a moment earlier, without messages and so at the bottom: this one takes
/// its place.
pub fn add_channel(s: &mut Snapshot, chat: Chat) {
    let id = chat.id.clone();
    s.chats.retain(|ch| ch.id != id);
    s.chats.push(chat);
    for sec in &mut s.sections {
        sec.chats.retain(|x| x != &id);
    }
    if let Some(sec) = s.sections.iter_mut().find(|sec| sec.id == "channels") {
        sec.chats.insert(0, id);
    }
}

/// Deletions [`remove`] remembers: the push of the user's own comes back within seconds.
const DELETED_KEPT: usize = 100;

/// Removes a deleted message. A reply lowers its thread's reply count even when the replies
/// are not loaded; a deleted parent takes its thread along, from the list and the thread pane.
/// Each deletion counts once: the user's own comes back from Telemost as a push.
pub fn remove(c: Ctx, chat: &ChatId, ts: Timestamp, thread_parent: Option<Timestamp>) {
    let key = (chat.clone(), ts);
    let first = c
        .deleted
        .try_update(|d| {
            if d.contains(&key) {
                return false;
            }
            if d.len() >= DELETED_KEPT {
                d.pop_front();
            }
            d.push_back(key);
            true
        })
        .unwrap_or(true);
    if !first {
        return;
    }
    let mut parent = thread_parent;
    c.snap.update(|s| {
        let removed = s
            .messages
            .iter()
            .position(|m| &m.chat_id == chat && m.ts == ts)
            .map(|pos| s.messages.remove(pos));
        parent = parent.or_else(|| removed.as_ref().and_then(|m| m.thread_parent));
        match parent {
            Some(parent) => {
                let participants = s.thread_participants(chat, parent);
                if let Some(p) = s.messages.iter_mut().find(|x| &x.chat_id == chat && x.ts == parent)
                    && let Some(t) = p.thread.as_mut()
                {
                    t.reply_count = t.reply_count.saturating_sub(1);
                    if let Some(r) = &removed
                        && !participants.contains(&r.author)
                    {
                        t.participants.retain(|a| a != &r.author);
                    }
                }
            }
            None => s
                .messages
                .retain(|m| !(&m.chat_id == chat && m.thread_parent == Some(ts))),
        }
    });
    c.threads.update(|list| match parent {
        Some(parent) => {
            if let Some(i) = list
                .iter()
                .position(|t| &t.parent.chat_id == chat && t.parent.ts == parent)
            {
                let t = &mut list[i];
                if let Some(th) = t.parent.thread.as_mut() {
                    th.reply_count = th.reply_count.saturating_sub(1);
                    if th.reply_count == 0 {
                        list.remove(i);
                    }
                }
            }
        }
        None => list.retain(|t| !(&t.parent.chat_id == chat && t.parent.ts == ts)),
    });
    if parent.is_none() && c.thread.get_untracked() == Some(ts) && c.current.with_untracked(|cur| cur == chat) {
        c.thread.set(None);
    }
}

/// A message Telemost sent again, with the thread details and the call length this session
/// knows from `old`.
pub(super) fn keep_known(mut m: Message, old: &[Message]) -> Message {
    let Some(known) = old.iter().find(|o| o.chat_id == m.chat_id && o.ts == m.ts) else {
        return m;
    };
    m.keep_reads(known);
    if let Some(t) = m.thread.as_mut()
        && let Some(known) = known.thread.as_ref()
    {
        t.keep_known(known);
    }
    if let Some(CallNote::Ended { seconds: s @ None, .. }) = m.call.as_mut()
        && let Some(CallNote::Ended {
            seconds: Some(lasted), ..
        }) = known.call
    {
        *s = Some(lasted);
    }
    m
}

/// The end of a group call with its length: Telemost reports the call ended, with its start,
/// a moment before the end note comes, which does not say how long it went.
pub(super) fn with_call_length(mut m: Message, meetings: &std::collections::HashMap<ChatId, Meeting>) -> Message {
    let Some(meeting) = meetings.get(&m.chat_id).filter(|meeting| meeting.ended) else {
        return m;
    };
    let lasted = call_seconds(&m, Some(meeting.started));
    if let Some(CallNote::Ended {
        url, seconds: s @ None, ..
    }) = m.call.as_mut()
        && *url == meeting.url
    {
        *s = lasted;
    }
    m
}
