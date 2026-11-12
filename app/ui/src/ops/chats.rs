//! Chats and their history: opening a chat, loading the latest and older pages, read counters,
//! back and forward, jumping to a message, marking read, leaving.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::Ctx;
use crate::bridge;
use crate::i18n::tr;

use super::snapshot::{document_visible, keep_known, reading};
use super::threads::open_thread;
use super::{failed, learn_users};

/// Selects a chat, loads its fresh history and marks it as read.
pub fn open_chat(c: Ctx, id: ChatId) {
    if !select_chat(c, id.clone(), true) {
        return;
    }
    spawn_local(async move {
        load_latest(c, &id).await;
    });
}

thread_local! {
    /// Chats whose latest page was loaded over the current connection: everything new in them
    /// has come as pushes since, so opening them again asks Telemost for nothing.
    static FRESH: std::cell::RefCell<std::collections::HashSet<ChatId>> = Default::default();
    /// Group chats where an own message may have been read since its counters came: the
    /// latest page is asked again for them when the chat is open and in sight.
    static RECOUNT: std::cell::RefCell<std::collections::HashSet<ChatId>> = Default::default();
    /// A recount is due shortly.
    static RECOUNT_SOON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Someone else read a chat: the read receipts follow, and a group whose earlier own messages
/// may have been read as well gets its counters again.
pub(super) fn peer_seen(c: Ctx, chat: ChatId, user: UserId, ts: Timestamp, reads: Option<Reads>) {
    // Reads of chats with nothing of the user's to show change nothing: no update then.
    let change = c
        .snap
        .try_maybe_update(|s| {
            let change = s.peer_seen(&chat, &user, ts, reads);
            (change.changed, change)
        })
        .unwrap_or_default();
    if change.recount {
        RECOUNT.with_borrow_mut(|r| r.insert(chat));
        recount_soon(c);
    }
}

/// Asks for the counters of the open chat in a moment, once for all the reads meanwhile.
pub fn recount_soon(c: Ctx) {
    if !c.live || RECOUNT_SOON.replace(true) {
        return;
    }
    set_timeout(
        move || {
            RECOUNT_SOON.set(false);
            recount(c);
        },
        std::time::Duration::from_secs(2),
    );
}

/// The latest page of the open chat again, if its counters may be stale, for the counters only.
fn recount(c: Ctx) {
    let id = c.current.get_untracked();
    if !document_visible() || !RECOUNT.with_borrow_mut(|r| r.remove(&id)) {
        return;
    }
    spawn_local(async move {
        match bridge::history(&id.0, None, None).await {
            Ok(page) => c.snap.update(|s| s.take_reads(&id, &page.messages, page.peer_seen)),
            Err(e) => leptos::logging::warn!("read counters: {e}"),
        }
    });
}

/// Forgets which chats are up to date: pushes may have been missed while offline.
pub(super) fn forget_fresh() {
    FRESH.with_borrow_mut(|f| f.clear());
}

thread_local! {
    /// Where the loaded history of each chat starts: the time of its oldest record, deleted
    /// messages and service records included. The next older page is asked for before it: after a
    /// run of deleted messages the oldest message shown would ask for the same page again.
    static CURSOR: std::cell::RefCell<std::collections::HashMap<ChatId, Timestamp>> = Default::default();
}

/// Where the loaded history of chat `id` starts, if a page of it has been loaded.
fn cursor(id: &ChatId) -> Option<Timestamp> {
    CURSOR.with_borrow(|c| c.get(id).copied())
}

fn set_cursor(id: &ChatId, ts: Option<Timestamp>) {
    CURSOR.with_borrow_mut(|c| match ts {
        Some(ts) => c.insert(id.clone(), ts),
        None => c.remove(id),
    });
}

/// Where the loaded part of message `m`'s chat starts: known for a feed message once a history
/// page has been loaded; thread replies come in pages of their own.
pub(super) fn loaded_from(m: &Message) -> Option<Timestamp> {
    m.thread_parent.is_none().then(|| cursor(&m.chat_id)).flatten()
}

/// The `before` of the next older history page of chat `id`: where its loaded history starts,
/// or the oldest message shown when no page has been loaded.
pub(super) fn older_page_before(s: &Snapshot, id: &ChatId, loaded: Option<Timestamp>) -> Option<Timestamp> {
    loaded.or_else(|| {
        s.messages
            .iter()
            .filter(|m| &m.chat_id == id && m.thread_parent.is_none())
            .map(|m| m.ts)
            .min()
    })
}

/// Where the history of a chat starts once `page`, asked for before `before` (`None` for the
/// latest page), has come: its oldest record, else its oldest message. `None` when the page has
/// nothing older than `before`, so there is nothing further back to ask for.
pub(super) fn page_start(before: Option<Timestamp>, page: &bridge::HistoryPage) -> Option<Timestamp> {
    let oldest = page.oldest.or_else(|| page.messages.iter().map(|m| m.ts).min())?;
    before.is_none_or(|b| oldest < b).then_some(oldest)
}

/// Adds a page of chat `id` asked for before `before`: its messages join the feed, the chat's
/// history now starts at the page's oldest record, and ends there when the page is the last one.
fn add_older_page(c: Ctx, id: &ChatId, before: Timestamp, page: bridge::HistoryPage) {
    let start = page_start(Some(before), &page);
    if start.is_some() {
        set_cursor(id, start);
    }
    if !page.more || start.is_none() {
        c.exhausted.update(|e| {
            e.insert(id.clone());
        });
    }
    c.snap.update(|s| {
        learn_users(s, page.users);
        for m in page.messages {
            if !s.messages.iter().any(|x| x.chat_id == m.chat_id && x.ts == m.ts) {
                s.messages.push(m);
            }
        }
    });
}

/// Makes the chat current (recording it in the navigation history if `record`). Returns `true`
/// if the chat history needs to be loaded from the server.
fn select_chat(c: Ctx, id: ChatId, record: bool) -> bool {
    c.hold_unread.set(None);
    c.pins_tab.set(false);
    let prev = c.current.get_untracked();
    if record && !prev.0.is_empty() && prev != id {
        c.nav_back.update(|b| {
            b.push(prev);
            if b.len() > 50 {
                b.remove(0);
            }
        });
        c.nav_forward.set(vec![]);
    }
    c.recent.update(|r| {
        r.retain(|x| x != &id);
        r.insert(0, id.clone());
        r.truncate(20);
    });
    // An opened chat is back in the sidebar.
    if c.hidden.with_untracked(|h| h.contains_key(&id)) {
        c.hidden.update(|h| {
            h.remove(&id);
        });
    }
    if c.view.get_untracked().is_home_page() {
        c.view.set(crate::app::ViewMode::Home);
    }
    c.current.set(id.clone());
    c.thread.set(None);
    c.editing.set(None);
    let read_on_open = c.settings.with_untracked(|s| s.mark_read_on_open);
    if read_on_open {
        c.snap.update(|s| {
            if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == id) {
                ch.unread = 0;
                ch.mentions = 0;
            }
        });
    }
    if c.live && FRESH.with_borrow(|f| f.contains(&id)) {
        // What `load_latest` would do after loading.
        recount_soon(c);
        if read_on_open {
            mark_read(c, id);
        }
        return false;
    }
    c.live
}

/// The latest history page of the chat instead of whatever was loaded.
pub(super) async fn load_latest(c: Ctx, id: &ChatId) {
    let read_on_open = c.settings.with_untracked(|s| s.mark_read_on_open);
    match bridge::history(&id.0, None, None).await {
        Ok(page) => {
            let full = !page.more;
            FRESH.with_borrow_mut(|f| f.insert(id.clone()));
            RECOUNT.with_borrow_mut(|r| r.remove(id));
            set_cursor(id, page_start(None, &page));
            c.snap.update(|s| {
                learn_users(s, page.users);
                if let Some(ch) = s.chats.iter_mut().find(|ch| &ch.id == id) {
                    ch.peer_seen_ts = ch.peer_seen_ts.max(page.peer_seen);
                }
                // History never has a ringing call: it stays, or "Join" would go.
                let old: Vec<Message> = s
                    .messages
                    .extract_if(.., |m| &m.chat_id == id && m.thread_parent.is_none() && !m.is_ringing())
                    .collect();
                s.messages
                    .extend(page.messages.into_iter().map(|m| keep_known(m, &old)));
            });
            c.exhausted.update(|e| {
                if full {
                    e.insert(id.clone());
                } else {
                    e.remove(id);
                }
            });
            fill_screen(c, id.clone());
            // Only what is in front of the user is read: not a chat in a hidden window after a
            // reconnection, nor one the user marked unread.
            if read_on_open && reading(c, id) {
                mark_read(c, id.clone());
            }
        }
        Err(e) => failed(c, tr().err_history, e),
    }
}

/// A short page may not fill the message list, and then there is no scrolling to load older
/// messages: load them until the list scrolls or the history ends. Runs after the list re-renders.
fn fill_screen(c: Ctx, id: ChatId) {
    set_timeout(
        move || {
            if c.current.get_untracked() != id {
                return;
            }
            let Some(el) = document().query_selector(".chat-feed").ok().flatten() else {
                return;
            };
            if el.scroll_height() <= el.client_height() {
                load_older(c);
            }
        },
        std::time::Duration::from_millis(16),
    );
}

/// "Back" / "Forward" through opened chats, like the arrows in Slack's top bar.
pub fn navigate(c: Ctx, back: bool) {
    let (from, to) = if back {
        (c.nav_back, c.nav_forward)
    } else {
        (c.nav_forward, c.nav_back)
    };
    let Some(id) = from.try_update(Vec::pop).flatten() else {
        return;
    };
    let cur = c.current.get_untracked();
    if !cur.0.is_empty() {
        to.update(|v| v.push(cur));
    }
    if select_chat(c, id.clone(), false) {
        spawn_local(async move { load_latest(c, &id).await });
    }
}

/// Opens the chat and scrolls to message `ts` (a thread reply opens in the thread panel), highlighting it.
/// If the message is old, loads history page by page until it is found.
pub fn jump_to_message(c: Ctx, chat: ChatId, ts: Timestamp, thread_parent: Option<Timestamp>) {
    let same = c.current.get_untracked() == chat;
    let load = if same {
        if c.view.get_untracked().is_home_page() {
            c.view.set(crate::app::ViewMode::Home);
        }
        false
    } else {
        select_chat(c, chat.clone(), true)
    };
    let target = thread_parent.unwrap_or(ts);
    let chat2 = chat.clone();
    let show = move || {
        if let Some(parent) = thread_parent {
            open_thread(c, parent);
        }
        c.highlight.set(Some((chat.clone(), ts)));
    };
    if !load && !c.live {
        return show();
    }
    spawn_local(async move {
        if load {
            load_latest(c, &chat2).await;
        }
        let has = |c: Ctx| {
            c.snap
                .with_untracked(|s| s.messages.iter().any(|m| m.chat_id == chat2 && m.ts == target))
        };
        let mut pages = 0;
        while c.live && !has(c) && pages < 20 && !c.exhausted.with_untracked(|e| e.contains(&chat2)) {
            let loaded = cursor(&chat2);
            let Some(before) = c.snap.with_untracked(|s| older_page_before(s, &chat2, loaded)) else {
                break;
            };
            match bridge::history(&chat2.0, Some(before), None).await {
                Ok(page) => add_older_page(c, &chat2, before, page),
                Err(e) => return failed(c, tr().err_older, e),
            }
            pages += 1;
        }
        show();
    });
}

/// "Mark as read" from the chat menu.
pub fn mark_chat_read(c: Ctx, id: ChatId) {
    if c.hold_unread.with_untracked(|h| h.as_ref() == Some(&id)) {
        c.hold_unread.set(None);
    }
    c.snap.update(|s| {
        if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == id) {
            ch.unread = 0;
            ch.mentions = 0;
        }
    });
    mark_read(c, id);
}

pub fn mark_read(c: Ctx, id: ChatId) {
    if !c.live {
        return;
    }
    spawn_local(async move {
        if let Err(e) = bridge::mark_read(&id.0).await {
            leptos::logging::warn!("mark_read: {e}");
        }
    });
}

/// Loads older messages of the current chat. Returns `false` if there is nothing more to load.
pub fn load_older(c: Ctx) -> bool {
    let id = c.current.get_untracked();
    if !c.live || c.loading_older.get_untracked() || c.exhausted.with_untracked(|e| e.contains(&id)) {
        return false;
    }
    let loaded = cursor(&id);
    let Some(before) = c.snap.with_untracked(|s| older_page_before(s, &id, loaded)) else {
        return false;
    };
    c.loading_older.set(true);
    spawn_local(async move {
        match bridge::history(&id.0, Some(before), None).await {
            Ok(page) => {
                add_older_page(c, &id, before, page);
                c.loading_older.set(false);
                fill_screen(c, id);
                return;
            }
            Err(e) => failed(c, tr().err_older, e),
        }
        c.loading_older.set(false);
    });
    true
}

/// Leaves the channel, removes it from the sidebar and opens the first remaining chat.
pub fn leave_chat(c: Ctx, id: ChatId) {
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::leave_chat(&id.0).await
        {
            return failed(c, tr().err_leave, e);
        }
        drop_chat(c, &id);
    });
}

/// Removes a chat the user is no longer in from the sidebar; if it was open, opens the first
/// remaining chat.
pub(super) fn drop_chat(c: Ctx, id: &ChatId) {
    if c.snap.with_untracked(|s| s.chat(id).is_none()) {
        return;
    }
    FRESH.with_borrow_mut(|f| f.remove(id));
    set_cursor(id, None);
    c.snap.update(|s| {
        s.chats.retain(|ch| &ch.id != id);
        s.messages.retain(|m| &m.chat_id != id);
        for sec in &mut s.sections {
            sec.chats.retain(|x| x != id);
        }
    });
    if &c.current.get_untracked() == id {
        let next = c
            .snap
            .with_untracked(|s| s.sections.iter().flat_map(|sec| sec.chats.first()).next().cloned());
        if let Some(n) = next {
            open_chat(c, n);
        }
    }
}
