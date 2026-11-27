//! Threads: the thread pane, its replies, and the followed threads of the "Threads" view.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::Ctx;
use crate::bridge;
#[cfg(doc)]
use crate::bridge::LiveEvent;
use crate::i18n::tr;

use super::chats::load_latest;
use super::snapshot::in_sight;
use super::{failed, learn_users};

thread_local! {
    /// Threads Telemost said the user stopped following ([`LiveEvent::ThreadLeft`]), each with
    /// the number of list loads ([`LOADS`]) started by then. A reply there does not bring one
    /// back, though the parent is the user's; Telemost's word does: the user's own reply or a
    /// mention there, [`LiveEvent::ThreadJoined`], or a list that has it and was asked for after
    /// the user left (the backend keeps out of its lists the threads left on its connection).
    static LEFT: std::cell::RefCell<std::collections::HashMap<(ChatId, Timestamp), u64>> = Default::default();
    /// Loads of the followed-thread list started ([`load_threads`]).
    static LOADS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many threads [`LEFT`] remembers; then it starts afresh.
const LEFT_MAX: usize = 200;

/// The user no longer follows the thread of message `parent`: it leaves the list.
pub(super) fn leave_thread(c: Ctx, chat_id: ChatId, parent: Timestamp) {
    c.threads
        .update(|list| list.retain(|t| t.parent.chat_id != chat_id || t.parent.ts != parent));
    LEFT.with_borrow_mut(|left| {
        if left.len() >= LEFT_MAX {
            left.clear();
        }
        left.insert((chat_id, parent), LOADS.get());
    });
}

/// Telemost made the user follow the thread of message `parent` again.
pub(super) fn rejoin_thread(chat_id: ChatId, parent: Timestamp) {
    LEFT.with_borrow_mut(|left| left.remove(&(chat_id, parent)));
}

/// Takes a list from the backend into account: the threads in it that the user left before
/// load number `load` went out are followed again; the others left stay out of it.
fn settle_left(threads: &mut Vec<FollowedThread>, load: u64) {
    LEFT.with_borrow_mut(|left| {
        threads.retain(|t| {
            let key = (t.parent.chat_id.clone(), t.parent.ts);
            match left.get(&key) {
                Some(by) if *by >= load => false,
                Some(_) => {
                    left.remove(&key);
                    true
                }
                None => true,
            }
        })
    });
}

pub fn open_thread(c: Ctx, parent: Timestamp) {
    c.profile.set(None);
    c.thread.set(Some(parent));
    let id = c.current.get_untracked();
    if !c.live {
        return read_thread(c, id, parent);
    }
    load_thread(c, id, parent);
}

/// Loads the replies of the open thread of message `parent` in chat `id` and marks it read.
pub(super) fn load_thread(c: Ctx, id: ChatId, parent: Timestamp) {
    spawn_local(async move {
        match bridge::history(&id.0, None, Some(parent), None).await {
            Ok(page) => {
                let last = page.messages.iter().max_by_key(|m| m.ts).map(|m| (m.ts, m.seq_no));
                // A thread that fits in one page gives the exact reply count, without the
                // deleted replies Telemost's count still has.
                let exact = (!page.more).then_some(page.messages.len() as u32);
                let mut summary = None;
                c.snap.update(|s| {
                    learn_users(s, page.users);
                    s.messages
                        .retain(|m| !(m.chat_id == id && m.thread_parent == Some(parent)));
                    s.messages.extend(page.messages);
                    let participants = s.thread_participants(&id, parent);
                    if let Some(p) = s.messages.iter_mut().find(|m| m.chat_id == id && m.ts == parent)
                        && let Some(t) = p.thread.as_mut()
                    {
                        if !participants.is_empty() {
                            t.participants = participants;
                        }
                        if let Some(n) = exact {
                            t.reply_count = n;
                        }
                        summary = Some(t.clone());
                    }
                });
                // The open thread is read, as in Slack.
                let mut unread = false;
                c.threads.update(|list| {
                    if let Some(t) = list
                        .iter_mut()
                        .find(|t| t.parent.chat_id == id && t.parent.ts == parent)
                    {
                        if let Some((ts, seq)) = last.filter(|(ts, _)| *ts >= t.last_ts) {
                            (t.last_ts, t.last_seq) = (ts, seq.max(t.last_seq));
                        }
                        if let Some(summary) = &summary {
                            t.parent.thread = Some(summary.clone());
                        }
                        unread = t.unread > 0;
                    }
                });
                if unread {
                    read_thread(c, id, parent);
                }
            }
            Err(e) => failed(c, tr().err_thread, e),
        }
    });
}

/// Followed threads, once per connection; the pushes keep the list current after that.
pub fn load_threads(c: Ctx) {
    if !c.live {
        // Demo: the threads the user replied in; others' later replies are unread.
        let list = c.snap.with_untracked(|s| {
            s.messages
                .iter()
                .filter(|p| p.thread.is_some())
                .filter_map(|p| {
                    let replies = s.thread_replies(&p.chat_id, p.ts);
                    let mine = replies.iter().rposition(|r| r.author == s.me)?;
                    Some(FollowedThread {
                        parent: p.clone(),
                        unread: replies[mine + 1..].iter().filter(|r| r.author != s.me).count() as u32,
                        last_ts: replies.last().map_or(p.ts, |r| r.ts),
                        last_seq: 0,
                        seen_seq: None,
                    })
                })
                .collect()
        });
        c.threads.set(list);
        return;
    }
    let load = LOADS.get() + 1;
    LOADS.set(load);
    spawn_local(async move {
        match bridge::followed_threads().await {
            Ok(mut t) => {
                c.snap.update(|s| learn_users(s, t.users));
                settle_left(&mut t.threads, load);
                c.threads.set(t.threads);
            }
            Err(e) => leptos::logging::warn!("threads: {e}"),
        }
    });
}

/// Takes a freshly fetched thread list. A thread that got replies since the fetch went out
/// keeps its newer local state; one only known locally (just replied in) stays.
pub(super) fn merge_threads(c: Ctx, mut fresh: Vec<FollowedThread>) {
    // The backend checks the list against the threads left on its connection as it sends it:
    // it is newer than any departure the UI has seen.
    settle_left(&mut fresh, u64::MAX);
    c.threads.update(|list| {
        let same =
            |a: &FollowedThread, b: &FollowedThread| a.parent.chat_id == b.parent.chat_id && a.parent.ts == b.parent.ts;
        let mut out = fresh;
        for t in out.iter_mut() {
            if let Some(old) = list.iter().find(|o| same(o, t))
                && old.last_ts > t.last_ts
            {
                (t.unread, t.last_ts, t.last_seq, t.seen_seq) = (old.unread, old.last_ts, old.last_seq, old.seen_seq);
                // Its count too: a thread the list has with all replies deleted got one since.
                t.parent.thread.clone_from(&old.parent.thread);
            }
        }
        for old in list.iter() {
            if !out.iter().any(|t| same(t, old)) {
                out.push(old.clone());
            }
        }
        out.sort_by_key(|t| std::cmp::Reverse(t.last_ts));
        *list = out;
    });
}

/// A new reply in the thread of message `parent`: the thread list follows it. Someone else's
/// reply is unread unless that thread is open in front of the user. A thread not in the list
/// yet joins it when the user replied, started it or is mentioned in the reply, as Telemost
/// subscribes them; when its parent is not loaded, the backend fetches the list again and
/// sends it as [`LiveEvent::Threads`].
pub(super) fn track_thread(c: Ctx, m: &Message, parent: Timestamp) {
    let (me, mentioned, parent_msg) = c.snap.with_untracked(|s| {
        let mentioned = s.mentions_me(&m.text);
        let parent_msg = s
            .messages
            .iter()
            .find(|x| x.chat_id == m.chat_id && x.ts == parent)
            .cloned();
        (s.me.clone(), mentioned, parent_msg)
    });
    let mine = m.author == me;
    let key = (m.chat_id.clone(), parent);
    let left = if mine || mentioned {
        LEFT.with_borrow_mut(|left| left.remove(&key));
        false
    } else {
        LEFT.with_borrow(|left| left.contains_key(&key))
    };
    let open =
        c.current.with_untracked(|cur| cur == &m.chat_id) && c.thread.get_untracked() == Some(parent) && in_sight();
    let mut read_now = false;
    c.threads.update(|list| {
        let at = list
            .iter()
            .position(|t| t.parent.chat_id == m.chat_id && t.parent.ts == parent);
        let at = match (at, parent_msg) {
            (Some(i), _) => {
                // One more reply, unless the fetched list counts it already. A thread added below
                // takes its parent from the chat, which counts the reply already.
                if m.seq_no == 0 || m.seq_no > list[i].last_seq {
                    list[i].add_reply(m.ts);
                }
                i
            }
            (None, Some(p)) if mine || mentioned || (p.author == me && !left) => {
                list.push(FollowedThread {
                    parent: p,
                    unread: 0,
                    last_ts: 0,
                    last_seq: 0,
                    seen_seq: None,
                });
                list.len() - 1
            }
            _ => return,
        };
        let t = &mut list[at];
        if m.ts >= t.last_ts {
            t.last_ts = m.ts;
            t.last_seq = t.last_seq.max(m.seq_no);
        }
        if mine {
            t.read();
        } else if open {
            read_now = t.unread > 0 || m.seq_no > 0;
            t.read();
        } else {
            t.unread = t.unread.saturating_add(1);
        }
        list.sort_by_key(|t| std::cmp::Reverse(t.last_ts));
    });
    if read_now && c.settings.with_untracked(|s| s.mark_read_on_open) {
        read_thread(c, m.chat_id.clone(), parent);
    }
}

/// Marks a followed thread read up to its last known reply.
pub fn read_thread(c: Ctx, chat: ChatId, parent: Timestamp) {
    let mut last = None;
    c.threads.update(|list| {
        if let Some(t) = list
            .iter_mut()
            .find(|t| t.parent.chat_id == chat && t.parent.ts == parent)
        {
            t.read();
            last = Some((t.last_ts, t.last_seq));
        }
    });
    let Some((ts, seq)) = last.filter(|(ts, seq)| *ts > 0 && *seq > 0) else {
        return;
    };
    if !c.live {
        return;
    }
    spawn_local(async move {
        if let Err(e) = bridge::mark_thread_read(&chat.0, parent, ts, seq).await {
            leptos::logging::warn!("mark_thread_read: {e}");
        }
    });
}

/// Opens a followed thread from the "Threads" view: the thread pane shows it next to the list.
/// The parent may come from a chat whose history is not loaded, so it is added if missing.
pub fn open_followed_thread(c: Ctx, parent: Message) {
    c.snap.update(|s| {
        if !s
            .messages
            .iter()
            .any(|m| m.chat_id == parent.chat_id && m.ts == parent.ts)
        {
            s.messages.push(parent.clone());
        }
    });
    c.editing.set(None);
    c.current.set(parent.chat_id.clone());
    open_thread(c, parent.ts);
    if !c.live {
        return;
    }
    // The chat is current now: Home, DMs or Activity show it, so it needs its history too.
    // Not marked read while the "Threads" page covers it.
    spawn_local(async move {
        load_latest(c, &parent.chat_id).await;
        c.snap.update(|s| {
            if !s
                .messages
                .iter()
                .any(|m| m.chat_id == parent.chat_id && m.ts == parent.ts)
            {
                s.messages.push(parent);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(ts: Timestamp) -> FollowedThread {
        FollowedThread {
            parent: Message {
                chat_id: ChatId("c".into()),
                ts,
                author: UserId("me".into()),
                text: String::new(),
                thread_parent: None,
                edited: false,
                important: false,
                reactions: vec![],
                thread: None,
                seq_no: 0,
                attachments: vec![],
                forwarded: vec![],
                call: None,
                system: None,
                media: None,
                reads: None,
            },
            unread: 0,
            last_ts: ts,
            last_seq: 0,
            seen_seq: None,
        }
    }

    fn left(ts: Timestamp) -> bool {
        LEFT.with_borrow(|l| l.contains_key(&(ChatId("c".into()), ts)))
    }

    /// A list asked for before the user left a thread keeps it out; one asked for after that
    /// has the word, and a thread in it is followed again.
    #[test]
    fn a_list_from_before_the_departure_does_not_bring_the_thread_back() {
        LOADS.set(3);
        LEFT.with_borrow_mut(|l| {
            l.clear();
            // Left while load 3 was out, and before load 2.
            l.insert((ChatId("c".into()), 1), 3);
            l.insert((ChatId("c".into()), 2), 1);
        });
        let mut list = vec![thread(1), thread(2), thread(5)];
        settle_left(&mut list, 3);
        assert_eq!(list.iter().map(|t| t.parent.ts).collect::<Vec<_>>(), [2, 5]);
        assert!(left(1) && !left(2));
        // A later list (or the backend's own, checked as it was sent) settles the first one too.
        let mut list = vec![thread(1)];
        settle_left(&mut list, u64::MAX);
        assert_eq!(list.len(), 1);
        assert!(!left(1));
    }
}
