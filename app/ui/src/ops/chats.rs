//! Chats and their history: opening a chat, loading the latest and older pages, read counters,
//! back and forward, jumping to a message, marking read, leaving.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::{ChatTab, Ctx};
use crate::bridge;
use crate::i18n::tr;

use super::snapshot::{document_visible, in_sight, keep_known, reading};
use super::threads::{open_thread, read_thread};
use super::{failed, learn_users};

/// Selects a chat, loads its fresh history and marks it as read.
pub fn open_chat(c: Ctx, id: ChatId) {
    // The chat already in front, its history loaded, stays as it is, as in Slack. Selecting it
    // again would set the signals of the chat view, and every `set` notifies: the list would be built again and jump
    // to the bottom, and the field would lose its caret.
    let in_front = c.current.get_untracked() == id
        && (!c.live || history_loaded(&id))
        && !c.view.get_untracked().is_home_page()
        && c.chat_tab.get_untracked() == ChatTab::Messages
        && !c.hidden.with_untracked(|h| h.contains_key(&id));
    if in_front {
        return;
    }
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
        match bridge::history(&id.0, None, None, None).await {
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

/// Whether the latest history page of chat `id` has been loaded over the current connection: only
/// then do older pages follow on from what is shown.
pub fn history_loaded(id: &ChatId) -> bool {
    FRESH.with_borrow(|f| f.contains(id))
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
    // A highlight left in the chat the user goes from would keep the next chat's list off its
    // bottom.
    if c.highlight.with_untracked(Option::is_some) {
        c.highlight.set(None);
    }
    c.chat_tab.set(ChatTab::Messages);
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
        c.snap.update(|s| s.read_chat(&id));
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
    match bridge::history(&id.0, None, None, None).await {
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
            // Only what is in front of the user is read: not a chat in a hidden or unfocused
            // window after a reconnection, nor one the user marked unread. The window gets it read
            // when it comes back in front ([`read_in_sight`]).
            if read_on_open {
                if reading(c, id) {
                    mark_read(c, id.clone());
                } else if c.current.with_untracked(|cur| cur == id) {
                    READ_LATER.with_borrow_mut(|r| *r = Some(id.clone()));
                }
            }
        }
        Err(e) => failed(c, tr().err_history, e),
    }
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
    jump(c, chat, ts, thread_parent, Some(20));
}

/// "View in chat" on the pinned message: as [`jump_to_message`], with no limit to the pages loaded
/// back to it, as the user asked to see that very message.
pub fn view_pinned(c: Ctx, chat: ChatId, ts: Timestamp) {
    jump(c, chat, ts, None, None);
}

/// [`jump_to_message`], loading at most `pages` older pages (`None`: no limit).
fn jump(c: Ctx, chat: ChatId, ts: Timestamp, thread_parent: Option<Timestamp>, pages: Option<u32>) {
    let same = c.current.get_untracked() == chat;
    let load = if same {
        if c.view.get_untracked().is_home_page() {
            c.view.set(crate::app::ViewMode::Home);
        }
        // The message is shown in the list: the "Files" tab gives way to it. ("Pinned" loads its
        // message this way and stays.)
        if c.chat_tab.get_untracked() == ChatTab::Files {
            c.chat_tab.set(ChatTab::Messages);
        }
        false
    } else {
        select_chat(c, chat.clone(), true)
    };
    let target = thread_parent.unwrap_or(ts);
    let chat2 = chat.clone();
    let shown = chat.clone();
    let show = move || {
        if let Some(parent) = thread_parent {
            open_thread(c, parent);
        }
        c.highlight.set(Some((shown.clone(), ts)));
    };
    if !load && !c.live {
        return show();
    }
    spawn_local(async move {
        // A chat opened anew shows its latest history, as from the sidebar.
        if load {
            load_latest(c, &chat).await;
        }
        // The user may leave the chat while its history loads: then loading stops and nothing
        // shows, or the thread would open in the chat they went to.
        let here = move |c: Ctx| c.current.get_untracked() == chat2;
        if load_back_to(c, &chat, target, pages, here).await.is_some() {
            show();
        }
    });
}

/// Loads the latest page of chat `chat` if it is not loaded over this connection and the message
/// is not there yet, then older pages until its message `ts` is there, the history ends, `pages`
/// pages have come (no limit with `None`) or `go_on` turns false. `Some(found)` when it is done, `None` when `go_on` turned false
/// or a page failed to load.
pub(super) async fn load_back_to(
    c: Ctx,
    chat: &ChatId,
    ts: Timestamp,
    pages: Option<u32>,
    go_on: impl Fn(Ctx) -> bool,
) -> Option<bool> {
    let has = |c: Ctx| {
        c.snap
            .with_untracked(|s| s.messages.iter().any(|m| &m.chat_id == chat && m.ts == ts))
    };
    // A message already there needs no request: offline it would only fail.
    if c.live && !has(c) && !history_loaded(chat) {
        load_latest(c, chat).await;
        if !history_loaded(chat) {
            return None;
        }
    }
    let mut loaded = 0;
    while c.live
        && go_on(c)
        && !has(c)
        && pages.is_none_or(|p| loaded < p)
        && !c.exhausted.with_untracked(|e| e.contains(chat))
    {
        let Some(before) = c.snap.with_untracked(|s| older_page_before(s, chat, cursor(chat))) else {
            break;
        };
        match bridge::history(&chat.0, Some(before), None, None).await {
            Ok(page) => add_older_page(c, chat, before, page),
            Err(e) => {
                failed(c, tr().err_older, e);
                return None;
            }
        }
        loaded += 1;
    }
    go_on(c).then(|| has(c))
}

thread_local! {
    /// The open chat whose latest page came while the window was out of sight: it is read when the
    /// window comes back, even with no unread messages counted.
    static READ_LATER: std::cell::RefCell<Option<ChatId>> = const { std::cell::RefCell::new(None) };
}

/// The window is back in front of the user: the open chat and thread, which counted what came
/// meanwhile as unread, are read now, as in Slack.
pub fn read_in_sight(c: Ctx) {
    let later = READ_LATER.with_borrow_mut(Option::take);
    if !c.settings.with_untracked(|s| s.mark_read_on_open) || !in_sight() {
        return;
    }
    let id = c.current.get_untracked();
    // The thread pane stays in sight beside a page of the Home view ("Threads") that covers the
    // chat, so it is read on its own, as a reply coming into it is ([`track_thread`]).
    if let Some(parent) = c.thread.get_untracked()
        && c.threads.with_untracked(|l| {
            l.iter()
                .any(|t| t.parent.chat_id == id && t.parent.ts == parent && t.unread > 0)
        })
    {
        read_thread(c, id.clone(), parent);
    }
    if !reading(c, &id) {
        return;
    }
    let had_unread = c
        .snap
        .try_maybe_update(|s| {
            let had = s.chat(&id).is_some_and(|ch| ch.unread > 0 || ch.mentions > 0);
            if had {
                s.read_chat(&id);
            }
            (had, had)
        })
        .unwrap_or(false);
    if had_unread || later.as_ref() == Some(&id) {
        mark_read(c, id);
    }
}

/// The pinned message `ts` of chat `id`, which the loaded history does not have: asked for alone,
/// not added to the feed. `None` when it is gone (deleted: any member of a group can do that).
pub async fn pinned_message(c: Ctx, id: &ChatId, ts: Timestamp) -> Result<Option<Message>, String> {
    // One record, the latest before one past its time: the message itself, if it is still there.
    let page = bridge::history(&id.0, Some(ts.saturating_add(1)), None, Some(1)).await?;
    if !page.users.is_empty() {
        c.snap.update(|s| learn_users(s, page.users));
    }
    Ok(page.messages.into_iter().find(|m| m.ts == ts && m.chat_id == *id))
}

/// "Mark as read" from the chat menu.
pub fn mark_chat_read(c: Ctx, id: ChatId) {
    if c.hold_unread.with_untracked(|h| h.as_ref() == Some(&id)) {
        c.hold_unread.set(None);
    }
    c.snap.update(|s| s.read_chat(&id));
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

thread_local! {
    /// Older pages being loaded, per chat: the token of the load and the failures in a row. A load
    /// that fails is asked again after a pause, the chat still loading meanwhile (its spinner
    /// stays), until a page comes or the user leaves the chat ([`stop_older`]). A timer or an
    /// answer of a load that has ended finds another token or none and changes nothing.
    static OLDER: std::cell::RefCell<std::collections::HashMap<ChatId, (u64, u32)>> = Default::default();
    static OLDER_TOKEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Loads older messages of the current chat. Returns `false` if there is nothing more to load.
/// While a load is going on, retries included, this does nothing: scrolling up does not cut a
/// pause short; the pauses start from a second again once the user has left the chat and come back.
pub fn load_older(c: Ctx) -> bool {
    let id = c.current.get_untracked();
    if !c.live || c.loading_older.with_untracked(|l| l.contains(&id)) || c.exhausted.with_untracked(|e| e.contains(&id))
    {
        return false;
    }
    if c.snap
        .with_untracked(|s| older_page_before(s, &id, cursor(&id)))
        .is_none()
    {
        return false;
    }
    c.loading_older.update(|l| {
        l.insert(id.clone());
    });
    let token = OLDER_TOKEN.get() + 1;
    OLDER_TOKEN.set(token);
    OLDER.with_borrow_mut(|o| o.insert(id.clone(), (token, 0)));
    ask_older(c, id, token);
    true
}

/// Whether load `token` of chat `id` is still going on.
fn older_going(id: &ChatId, token: u64) -> bool {
    OLDER.with_borrow(|o| o.get(id).is_some_and(|&(t, _)| t == token))
}

/// Asks for the older page of chat `id` for load `token`, again after a pause if it fails.
fn ask_older(c: Ctx, id: ChatId, token: u64) {
    // The history may have ended meanwhile another way (a jump to an old message, the latest page
    // after a reconnection): nothing to ask then.
    let before = c.snap.with_untracked(|s| older_page_before(s, &id, cursor(&id)));
    let Some(before) = before.filter(|_| !c.exhausted.with_untracked(|e| e.contains(&id))) else {
        return end_older(c, &id, token);
    };
    spawn_local(async move {
        match bridge::history(&id.0, Some(before), None, None).await {
            // The page goes on from the list only if the list still starts where it was asked
            // from: meanwhile the latest page may have replaced the list, or a jump to a message
            // or another load may have taken it further back. Else it is dropped (it would leave a
            // gap, or move the start back), and asked again from where the list starts now.
            Ok(_) if c.snap.with_untracked(|s| older_page_before(s, &id, cursor(&id))) != Some(before) => {
                if older_going(&id, token) {
                    ask_older(c, id, token);
                }
            }
            // A page is kept even if the user has left the chat meanwhile.
            Ok(page) => {
                add_older_page(c, &id, before, page);
                end_older(c, &id, token);
            }
            // No toast: the spinner stays while the page is asked again, whatever the reason
            // (the server, the connection).
            Err(e) => {
                leptos::logging::warn!("older page: {e}");
                let fails = OLDER.with_borrow_mut(|o| match o.get_mut(&id) {
                    Some((t, n)) if *t == token => {
                        *n += 1;
                        Some(*n)
                    }
                    _ => None,
                });
                let Some(fails) = fails else { return };
                set_timeout(
                    move || {
                        if older_going(&id, token) {
                            ask_older(c, id, token);
                        }
                    },
                    older_retry_pause(fails),
                );
            }
        }
    });
}

/// Ends load `token` of chat `id`, if it is still going on.
fn end_older(c: Ctx, id: &ChatId, token: u64) {
    if older_going(id, token) {
        stop_older(c, id);
    }
}

/// Ends loading older messages of chat `id`, retries included: a page has come, the user has left
/// the chat, or the chat is gone. Its spinner goes; coming back, the list asks again when scrolled
/// up, or by itself if it is short.
pub fn stop_older(c: Ctx, id: &ChatId) {
    if OLDER.with_borrow_mut(|o| o.remove(id).is_some()) {
        c.loading_older.try_update(|l| {
            l.remove(id);
        });
    }
}

/// How long to wait before asking again for an older page after `fails` failures in a row:
/// 1, 2, 4, 8, 16 seconds, then 30. Without a pause a short list would ask every frame.
fn older_retry_pause(fails: u32) -> std::time::Duration {
    std::time::Duration::from_secs((1u64 << fails.saturating_sub(1).min(5)).min(30))
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
    stop_older(c, id);
    crate::app::drafts::forget_chat(c, id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_retry_pause_grows_to_half_a_minute() {
        let secs: Vec<u64> = (1..=8).map(|n| older_retry_pause(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(older_retry_pause(u32::MAX).as_secs(), 30);
    }
}
