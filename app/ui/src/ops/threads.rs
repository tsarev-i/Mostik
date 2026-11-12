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
use super::snapshot::document_visible;
use super::{failed, learn_users};

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
        match bridge::history(&id.0, None, Some(parent)).await {
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
                    })
                })
                .collect()
        });
        c.threads.set(list);
        return;
    }
    spawn_local(async move {
        match bridge::followed_threads().await {
            Ok(t) => {
                c.snap.update(|s| learn_users(s, t.users));
                c.threads.set(t.threads);
            }
            Err(e) => leptos::logging::warn!("threads: {e}"),
        }
    });
}

/// Takes a freshly fetched thread list. A thread that got replies since the fetch went out
/// keeps its newer local state; one only known locally (just replied in) stays.
pub(super) fn merge_threads(c: Ctx, fresh: Vec<FollowedThread>) {
    c.threads.update(|list| {
        let same =
            |a: &FollowedThread, b: &FollowedThread| a.parent.chat_id == b.parent.chat_id && a.parent.ts == b.parent.ts;
        let mut out = fresh;
        for t in out.iter_mut() {
            if let Some(old) = list.iter().find(|o| same(o, t))
                && old.last_ts > t.last_ts
            {
                (t.unread, t.last_ts, t.last_seq) = (old.unread, old.last_ts, old.last_seq);
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
    let open = c.current.with_untracked(|cur| cur == &m.chat_id)
        && c.thread.get_untracked() == Some(parent)
        && document_visible();
    let mut read_now = false;
    c.threads.update(|list| {
        let at = list
            .iter()
            .position(|t| t.parent.chat_id == m.chat_id && t.parent.ts == parent);
        let at = match (at, parent_msg) {
            (Some(i), _) => i,
            (None, Some(p)) if mine || mentioned || p.author == me => {
                list.push(FollowedThread {
                    parent: p,
                    unread: 0,
                    last_ts: 0,
                    last_seq: 0,
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
            t.unread = 0;
        } else if open {
            read_now = t.unread > 0 || m.seq_no > 0;
            t.unread = 0;
        } else {
            t.unread += 1;
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
            t.unread = 0;
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
