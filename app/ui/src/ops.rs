//! UI operations. In live mode they go through the Tauri backend, in demo mode they stay local.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::{Ctx, Phase};
use crate::bridge::{self, LiveEvent};
use crate::i18n::tr;

fn report(c: Ctx, e: String) {
    leptos::logging::warn!("{e}");
    c.error.set(Some(e));
}

fn now_mcs() -> Timestamp {
    (js_sys::Date::now() * 1000.0) as Timestamp
}

/// Authors that came with a history page: new ones are added, a known one keeps its data and only
/// gets what it lacked (a push may have brought the bare `From` without a photo).
fn learn_users(s: &mut Snapshot, users: Vec<User>) {
    for u in users {
        match s.users.iter_mut().find(|x| x.id == u.id) {
            Some(known) => {
                if !known.has_name() && u.has_name() {
                    known.display_name = u.display_name;
                }
                known.avatar = known.avatar.take().or(u.avatar);
                known.title = known.title.take().or(u.title);
            }
            None => s.users.push(u),
        }
    }
}

/// Inserts or replaces a message; keeps the thread and unread counters up to date.
pub fn upsert(c: Ctx, m: Message, author: Option<User>) {
    let current = c.current.get_untracked();
    let open_thread = c.thread.get_untracked();
    let is_new = c
        .snap
        .with_untracked(|s| !s.messages.iter().any(|x| x.chat_id == m.chat_id && x.ts == m.ts));
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
            *old = Message {
                thread,
                reactions,
                seq_no,
                ..m.clone()
            };
            return;
        }
        let mine = m.author == s.me;
        let viewing = current == m.chat_id
            && document_visible()
            && c.hold_unread.with_untracked(|h| h.as_ref() != Some(&m.chat_id));
        // Only an unread feed message from someone else can add a mention. A ringing call is not
        // a message: Telemost counts only the note that the call ended.
        let counts = !mine && !viewing && m.thread_parent.is_none() && !m.is_ringing();
        let mentioned = counts && s.mentions_me(&m.text);
        if let Some(parent) = m.thread_parent {
            if let Some(p) = s.messages.iter_mut().find(|x| x.chat_id == m.chat_id && x.ts == parent) {
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
    });
    if let Some(parent) = m.thread_parent
        && is_new
    {
        track_thread(c, &m, parent);
    }
    let viewing_reply = m.thread_parent.is_some() && open_thread == m.thread_parent;
    let read_on_open = c.settings.with_untracked(|s| s.mark_read_on_open);
    let held = c.hold_unread.with_untracked(|h| h.as_ref() == Some(&m.chat_id));
    if c.live
        && read_on_open
        && !held
        && m.chat_id == current
        && !c.view.get_untracked().is_home_page()
        && (m.thread_parent.is_none() || viewing_reply)
        && document_visible()
    {
        mark_read(c, m.chat_id);
    }
}

fn document_visible() -> bool {
    !document().hidden()
}

/// Chats in the sections ordered by recency, as in Telemost.
fn sort_sections(s: &mut Snapshot) {
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

/// Removes a deleted message. A reply lowers its thread's reply count even when the replies
/// are not loaded; a deleted parent takes its thread along, from the list and the thread pane.
/// Each deletion counts once: the user's own comes back from Telemost as a push.
pub fn remove(c: Ctx, chat: &ChatId, ts: Timestamp, thread_parent: Option<Timestamp>) {
    let first = c.deleted.try_update(|d| d.insert((chat.clone(), ts))).unwrap_or(true);
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
            upsert(c, *message, author)
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
        LiveEvent::Typing { chat_id, user } => {
            let until = js_sys::Date::now() + 5000.0;
            c.typing.update(|t| {
                t.retain(|(ch, u, _)| !(ch == &chat_id && u == &user));
                t.push((chat_id, user, until));
            });
        }
        LiveEvent::Online { user, online } => {
            if online {
                LAST_HEARTBEAT.with_borrow_mut(|m| m.insert(user.clone(), js_sys::Date::now()));
            }
            c.snap.update(|s| {
                if let Some(u) = s.users.iter_mut().find(|u| u.id == user) {
                    u.presence = if online { Presence::Online } else { Presence::Offline };
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
        LiveEvent::Connection { online } => c.online.set(online),
        LiveEvent::Users { users } => c.snap.update(|s| learn_users(s, users)),
        LiveEvent::Threads { threads, users } => {
            c.snap.update(|s| learn_users(s, users));
            merge_threads(c, threads);
        }
    }
}

/// Selects a chat, loads its fresh history and marks it as read.
pub fn open_chat(c: Ctx, id: ChatId) {
    if !select_chat(c, id.clone(), true) {
        return;
    }
    spawn_local(async move {
        load_latest(c, &id).await;
    });
}

/// A random number, for new ids.
pub fn random_u64() -> u64 {
    let half = || (js_sys::Math::random() * 4_294_967_296.0) as u64;
    (half() << 32) | half()
}

/// Changes the sidebar sections of the current organization and writes them to Telemost, where
/// they are chat folders. Shown at once; put back if Telemost refuses.
pub fn change_folders(c: Ctx, change: impl FnOnce(&mut crate::folders::Folders)) {
    use crate::folders;
    let before = c.folders.get_untracked();
    let mut list = before.clone();
    change(&mut list);
    if list == before {
        return;
    }
    let org = c.snap.with_untracked(|s| crate::app::org_key(s, c.org.get_untracked()));
    let put = move |list: &folders::Folders| {
        let mine: Vec<_> = list.iter().map(|f| folders::to_telemost(f, &org)).collect();
        c.snap.update(|s| {
            s.folders.retain(|f| f.org != org);
            s.folders.extend(mine.iter().cloned());
        });
        mine
    };
    let mine = put(&list);
    if !c.live {
        return;
    }
    let org = c.snap.with_untracked(|s| crate::app::org_key(s, c.org.get_untracked()));
    spawn_local(async move {
        if let Err(e) = bridge::save_chat_folders(&org, &mine).await {
            put(&before);
            report(c, format!("{}: {e}", tr().err_settings));
        }
    });
}

/// Reads the chat folders afresh: they may have changed in Telemost.
pub fn refresh_folders(c: Ctx) {
    spawn_local(async move {
        if let Ok(list) = bridge::chat_folders().await
            && c.snap.with_untracked(|s| s.folders != list)
        {
            c.snap.update(|s| s.folders = list);
        }
    });
}

/// Hides the chat from the sidebar until something new comes in it.
pub fn hide_chat(c: Ctx, id: ChatId) {
    let Some(last) = c.snap.with_untracked(|s| s.chat(&id).map(|ch| ch.last_ts)) else {
        return;
    };
    c.hidden.update(|h| {
        h.insert(id, last);
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
    c.live
}

/// The latest history page of the chat instead of whatever was loaded.
async fn load_latest(c: Ctx, id: &ChatId) {
    let read_on_open = c.settings.with_untracked(|s| s.mark_read_on_open);
    match bridge::history(&id.0, None, None).await {
        Ok(page) => {
            let full = !page.more;
            c.snap.update(|s| {
                learn_users(s, page.users);
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
            if read_on_open && !c.view.get_untracked().is_home_page() {
                mark_read(c, id.clone());
            }
        }
        Err(e) => report(c, format!("{}: {e}", tr().err_history)),
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
            let oldest = c.snap.with_untracked(|s| {
                s.messages
                    .iter()
                    .filter(|m| m.chat_id == chat2 && m.thread_parent.is_none())
                    .map(|m| m.ts)
                    .min()
            });
            let Some(oldest) = oldest else { break };
            match bridge::history(&chat2.0, Some(oldest), None).await {
                Ok(page) => {
                    if !page.more {
                        c.exhausted.update(|e| {
                            e.insert(chat2.clone());
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
                Err(e) => return report(c, format!("{}: {e}", tr().err_older)),
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
    let oldest = c.snap.with_untracked(|s| {
        s.messages
            .iter()
            .filter(|m| m.chat_id == id && m.thread_parent.is_none())
            .map(|m| m.ts)
            .min()
    });
    let Some(oldest) = oldest else { return false };
    c.loading_older.set(true);
    spawn_local(async move {
        match bridge::history(&id.0, Some(oldest), None).await {
            Ok(page) => {
                if !page.more {
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
                c.loading_older.set(false);
                fill_screen(c, id);
                return;
            }
            Err(e) => report(c, format!("{}: {e}", tr().err_older)),
        }
        c.loading_older.set(false);
    });
    true
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
fn load_thread(c: Ctx, id: ChatId, parent: Timestamp) {
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
            Err(e) => report(c, format!("{}: {e}", tr().err_thread)),
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

/// A message Telemost sent again, with the thread details this session knows from `old`.
fn keep_known(mut m: Message, old: &[Message]) -> Message {
    if let Some(t) = m.thread.as_mut()
        && let Some(known) = old
            .iter()
            .find(|o| o.chat_id == m.chat_id && o.ts == m.ts)
            .and_then(|o| o.thread.as_ref())
    {
        t.keep_known(known);
    }
    m
}

/// Takes a freshly fetched thread list. A thread that got replies since the fetch went out
/// keeps its newer local state; one only known locally (just replied in) stays.
fn merge_threads(c: Ctx, fresh: Vec<FollowedThread>) {
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
fn track_thread(c: Ctx, m: &Message, parent: Timestamp) {
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

/// People the text may name in `@Name` mentions, as `(name, id)` for
/// [`mostik_core::markup::names_to_ids`]: the ones picked from suggestions, then everyone known
/// whose name is unique.
fn mention_targets(c: Ctx) -> Vec<(String, String)> {
    let mut known: Vec<(String, String)> = c
        .picked
        .with_untracked(|p| p.iter().map(|(name, id)| (name.clone(), id.0.clone())).collect());
    c.snap.with_untracked(|s| {
        for u in s.users.iter().filter(|u| u.has_name()) {
            let unique = s.users.iter().filter(|x| x.display_name == u.display_name).count() == 1;
            if unique && !known.iter().any(|(name, _)| name == &u.display_name) {
                known.push((u.display_name.clone(), u.id.0.clone()));
            }
        }
    });
    known
}

/// Text to send: `@Name` mentions become `@<guid>`, the way Telemost writes them.
fn with_mention_ids(c: Ctx, text: &str) -> String {
    if !text.contains('@') {
        return text.to_owned();
    }
    mostik_core::markup::names_to_ids(text, &mention_targets(c))
}

/// What the composer replies to: a message of its chat or thread, and the quoted fragment.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplyTarget {
    pub msg: Message,
    pub quote: Option<String>,
}

impl ReplyTarget {
    /// The original as the reply shows it above its text.
    fn forwarded(&self, c: Ctx) -> Forwarded {
        let m = &self.msg;
        Forwarded {
            author: m.author.clone(),
            author_name: c
                .snap
                .with_untracked(|s| s.user(&m.author).map(|u| u.display_name.clone()))
                .unwrap_or_default(),
            ts: m.ts,
            chat_id: Some(m.chat_id.clone()),
            text: m.text.clone(),
            attachments: m.attachments.clone(),
            quote: self.quote.clone(),
            reply: true,
        }
    }
}

/// Starts a reply to `msg` (quoting `quote`, when given) in the composer of its chat or thread.
pub fn start_reply(c: Ctx, msg: Message, quote: Option<String>) {
    c.editing.set(None);
    let quote = quote.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty());
    c.replying.set(Some(ReplyTarget { msg, quote }));
}

pub async fn send(
    c: Ctx,
    chat: ChatId,
    text: String,
    thread: Option<Timestamp>,
    reply: Option<ReplyTarget>,
) -> Result<(), String> {
    let text = with_mention_ids(c, &text);
    let me = c.snap.with_untracked(|s| s.me.clone());
    let reply_ref = reply.as_ref().map(|r| bridge::ReplyRef {
        ts: r.msg.ts,
        quote: r.quote.as_deref(),
    });
    let mut message = if c.live {
        bridge::send_message(&chat.0, &text, thread, reply_ref).await?
    } else {
        Message {
            chat_id: chat,
            ts: now_mcs(),
            author: me,
            text,
            thread_parent: thread,
            edited: false,
            important: false,
            reactions: vec![],
            thread: None,
            seq_no: 0,
            attachments: vec![],
            forwarded: vec![],
            call: None,
        }
    };
    if let Some(r) = &reply {
        message.forwarded = vec![r.forwarded(c)];
    }
    upsert(c, message, None);
    Ok(())
}

/// Sends the picked files with a caption; not available in demo mode.
pub async fn send_files(c: Ctx, chat: ChatId, text: String, thread: Option<Timestamp>) -> Result<(), String> {
    if !c.live {
        return Err(tr().only_in_app_files.into());
    }
    let paths: Vec<String> = c.pending_files.get_untracked().into_iter().map(|f| f.path).collect();
    let sent = bridge::send_files(&chat.0, thread, paths, &with_mention_ids(c, &text)).await?;
    c.pending_files.set(vec![]);
    for m in sent {
        upsert(c, m, None);
    }
    Ok(())
}

pub async fn edit(c: Ctx, m: Message, text: String) -> Result<(), String> {
    let text = with_mention_ids(c, &text);
    if c.live {
        bridge::edit_message(&m, &text).await?;
    }
    upsert(
        c,
        Message {
            text,
            edited: true,
            ..m
        },
        None,
    );
    Ok(())
}

pub fn delete(c: Ctx, m: Message) {
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::delete_message(&m).await
        {
            return report(c, format!("{}: {e}", tr().err_delete));
        }
        remove(c, &m.chat_id, m.ts, m.thread_parent);
    });
}

/// Adds reaction `kind`, or removes it if it is already mine. Telemost keeps one reaction per
/// person on a message, so a new one replaces the one the user had.
pub fn toggle_reaction(c: Ctx, m: &Message, kind: u64) {
    let add = !m.reactions.iter().any(|r| r.kind == kind && r.mine);
    let m = m.clone();
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::set_reaction(&m, kind, add).await
        {
            return report(c, format!("{}: {e}", tr().err_reaction));
        }
        // The resulting reactions arrive as a push; update locally right away so as not to wait.
        c.snap.update(|s| {
            let me = s.me.clone();
            let Some(msg) = s.messages.iter_mut().find(|x| x.chat_id == m.chat_id && x.ts == m.ts) else {
                return;
            };
            if add {
                for r in msg.reactions.iter_mut().filter(|r| r.kind != kind && r.mine) {
                    r.count = r.count.saturating_sub(1);
                    r.mine = false;
                    r.users.retain(|u| u != &me);
                }
            }
            match msg.reactions.iter_mut().find(|r| r.kind == kind) {
                Some(r) if add && !r.mine => {
                    r.count += 1;
                    r.mine = true;
                    r.users.push(me.clone());
                }
                Some(r) if !add && r.mine => {
                    r.count = r.count.saturating_sub(1);
                    r.mine = false;
                    r.users.retain(|u| u != &me);
                }
                Some(_) => {}
                None if add => msg.reactions.push(Reaction {
                    kind,
                    emoji: reactions::emoji(kind).to_owned(),
                    count: 1,
                    mine: true,
                    users: vec![me.clone()],
                }),
                None => {}
            }
            msg.reactions.retain(|r| r.count > 0);
        });
    });
}

pub fn toggle_important(c: Ctx, m: &Message) {
    let m = m.clone();
    let flag = !m.important;
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::set_important(&m, flag).await
        {
            return report(c, format!("{}: {e}", tr().err_important));
        }
        c.snap.update(|s| {
            if let Some(msg) = s.messages.iter_mut().find(|x| x.chat_id == m.chat_id && x.ts == m.ts) {
                msg.important = flag;
            }
        });
    });
}

/// Reports typing at most once every 4 seconds.
pub fn typing(c: Ctx, chat: ChatId) {
    if !c.live {
        return;
    }
    let now = js_sys::Date::now();
    if now - c.last_typing.get_untracked() < 4000.0 {
        return;
    }
    c.last_typing.set(now);
    spawn_local(async move {
        let _ = bridge::typing(&chat.0).await;
    });
}

/// A call in a chat, like a Slack huddle: the chat meeting opens in the call window.
/// If this chat's call is already open, its window comes to the front.
pub fn start_call(c: Ctx, chat: ChatId) {
    if !c.live {
        return report(c, tr().only_in_app_calls.into());
    }
    if let Some(current) = c.call.get_untracked() {
        // One call at a time, as in Slack: bring the current one back instead.
        focus_call(c);
        if current.chat_id.as_deref() != Some(chat.0.as_str()) {
            report(c, tr().call_busy.into());
        }
        return;
    }
    let label = c.snap.with_untracked(|s| {
        s.chat(&chat).map_or_else(String::new, |ch| {
            let title = crate::app::display_title(s, ch);
            match ch.kind {
                ChatKind::Group { public: true } | ChatKind::Broadcast => format!("#{title}"),
                _ => title,
            }
        })
    });
    let url = c.active_calls.with_untracked(|a| a.get(&chat).cloned());
    c.notice.set(Some(tr().call_opening.into()));
    spawn_local(async move {
        if let Err(e) = bridge::start_call(&chat.0, &label, url.as_deref()).await {
            c.notice.set(None);
            report(c, format!("{}: {e}", tr().err_call));
        }
    });
}

/// The call button of a chat whose call is already open: bring its window back.
pub fn focus_call(c: Ctx) {
    spawn_local(async move {
        if let Err(e) = bridge::focus_call().await {
            report(c, format!("{}: {e}", tr().err_call));
        }
    });
}

/// "Leave" in the sidebar call bar: the bar goes away when the backend reports the call closed.
pub fn leave_call(c: Ctx) {
    spawn_local(async move {
        if let Err(e) = bridge::leave_call().await {
            report(c, format!("{}: {e}", tr().err_call));
        }
    });
}

/// "Call" in a profile: a meeting in the direct chat with the person (the chat is created if needed).
pub fn call_user(c: Ctx, user: UserId) {
    if !c.live {
        return report(c, tr().only_in_app_calls.into());
    }
    let existing = c.snap.with_untracked(|s| {
        s.chats
            .iter()
            .find(|ch| {
                matches!(&ch.kind, ChatKind::Direct { peer } if peer == &user)
                    || (ch.kind == ChatKind::Notes && user == s.me)
            })
            .map(|ch| ch.id.clone())
    });
    if let Some(id) = existing {
        return start_call(c, id);
    }
    spawn_local(async move {
        match bridge::open_dm(&user.0).await {
            Ok(chat) => start_call(c, chat.id),
            Err(e) => report(c, format!("{}: {e}", tr().err_open_dm)),
        }
    });
}

/// Message link, like "Copy link" in Slack: the web client route `/chats/<chat>/<time>`.
pub fn message_link(m: &Message) -> String {
    let ts = m.thread_parent.unwrap_or(m.ts);
    format!(
        "{LINK_BASE}/chats/{}/{ts}",
        String::from(js_sys::encode_uri_component(&m.chat_id.0))
    )
}

const LINK_BASE: &str = "https://messenger.360.yandex.ru";

/// Telemost message link → `(chat, time)`, so that it opens in Mostik rather than in the browser.
fn parse_message_link(url: &str) -> Option<(ChatId, Timestamp)> {
    let (chat, ts) = split_message_link(url)?;
    let chat = String::from(js_sys::decode_uri_component(chat).ok()?);
    Some((ChatId(chat), ts))
}

/// `(encoded chat, time)` of a message link. Accepts `/chats/<chat>/<time>` with an optional
/// `/chat` prefix and an optional `#` route, as the web clients write it.
fn split_message_link(url: &str) -> Option<(&str, Timestamp)> {
    let rest = ["https://messenger.360.yandex.ru", "https://telemost.360.yandex.ru"]
        .iter()
        .find_map(|base| url.strip_prefix(base))?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    // Only a whole `chat` segment is a prefix: `chats/…` must stay intact.
    let rest = rest
        .strip_prefix("chat/")
        .or_else(|| rest.strip_prefix("chat#"))
        .unwrap_or(rest);
    let rest = rest.trim_start_matches('#').trim_start_matches('/');
    let rest = rest.strip_prefix("chats/")?;
    let (chat, ts) = rest.split_once('/')?;
    (!chat.is_empty()).then_some(())?;
    Some((chat, ts.trim_end_matches('/').parse().ok()?))
}

/// Plays a notification sound; the demo in a browser stays silent.
pub fn play_sound(c: Ctx, id: String) {
    if c.live {
        spawn_local(async move {
            let _ = bridge::play_sound(&id).await;
        });
    }
}

/// What a chat notifies about: everything, mentions only or nothing.
pub fn set_notify_level(c: Ctx, id: ChatId, level: NotifyLevel) {
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::set_notify_level(&id.0, level).await
        {
            return report(c, format!("{}: {e}", tr().err_settings));
        }
        c.snap.update(|s| {
            if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == id) {
                ch.set_notify_level(level);
            }
        });
        c.notice.set(Some(tr().notify_level_notice(level)));
    });
}

pub fn is_starred(c: Ctx, id: &ChatId) -> bool {
    c.snap.with_untracked(|s| {
        s.sections
            .iter()
            .any(|sec| sec.id == "starred" && sec.chats.contains(id))
    })
}

/// "Star" / "Unstar": pinned chats in Telemost.
pub fn toggle_star(c: Ctx, id: ChatId) {
    let starred = !is_starred(c, &id);
    spawn_local(async move {
        let list: Vec<ChatId> = if c.live {
            match bridge::set_starred(&id.0, starred).await {
                Ok(list) => list.into_iter().map(ChatId).collect(),
                Err(e) => return report(c, format!("{}: {e}", tr().err_settings)),
            }
        } else {
            let mut list = c.snap.with_untracked(|s| {
                s.sections
                    .iter()
                    .find(|sec| sec.id == "starred")
                    .map(|sec| sec.chats.clone())
                    .unwrap_or_default()
            });
            list.retain(|x| x != &id);
            if starred {
                list.push(id.clone());
            }
            list
        };
        c.snap.update(|s| apply_starred(s, list));
    });
}

/// Sorts chats into sections: starred ones into "Starred", the rest by kind.
fn apply_starred(s: &mut Snapshot, starred: Vec<ChatId>) {
    let starred: Vec<ChatId> = starred.into_iter().filter(|id| s.chat(id).is_some()).collect();
    let old: Vec<ChatId> = s
        .sections
        .iter()
        .find(|sec| sec.id == "starred")
        .map(|sec| sec.chats.clone())
        .unwrap_or_default();
    s.sections.retain(|sec| sec.id != "starred");
    for sec in &mut s.sections {
        sec.chats.retain(|id| !starred.contains(id));
    }
    // Unstarred chats return to their own section.
    for id in old.iter().filter(|id| !starred.contains(id)) {
        let Some(ch) = s.chat(id) else { continue };
        let target = crate::folders::home_section(ch);
        if let Some(sec) = s.sections.iter_mut().find(|sec| sec.id == target)
            && !sec.chats.contains(id)
        {
            sec.chats.push(id.clone());
        }
    }
    if !starred.is_empty() {
        let title = tr().section("starred", "").to_owned();
        s.sections.insert(
            0,
            Section {
                id: "starred".into(),
                title,
                chats: starred,
            },
        );
    }
    sort_sections(s);
}

/// "Mark unread" from this message, as in Slack.
pub fn mark_unread(c: Ctx, m: &Message) {
    let m = m.clone();
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::mark_unread(&m.chat_id.0, m.ts, m.seq_no).await
        {
            return report(c, format!("{}: {e}", tr().err_settings));
        }
        c.snap.update(|s| {
            let n = s
                .messages
                .iter()
                .filter(|x| x.chat_id == m.chat_id && x.thread_parent.is_none() && x.ts >= m.ts)
                .count() as u32;
            if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == m.chat_id) {
                ch.unread = n.max(1);
            }
        });
        // As in Slack: the chat stays unread until you come back to it.
        c.hold_unread.set(Some(m.chat_id.clone()));
    });
}

/// Pins a message in the chat or unpins it (`pin = false`).
pub fn pin_message(c: Ctx, m: &Message, pin: bool) {
    let m = m.clone();
    spawn_local(async move {
        let ts = pin.then_some(m.ts);
        if c.live
            && let Err(e) = bridge::pin_message(&m.chat_id.0, ts).await
        {
            return report(c, format!("{}: {e}", tr().err_pin));
        }
        set_pinned(c, &m.chat_id, ts);
    });
}

fn set_pinned(c: Ctx, chat: &ChatId, ts: Option<Timestamp>) {
    c.snap.update(|s| {
        if let Some(ch) = s.chats.iter_mut().find(|ch| &ch.id == chat) {
            ch.pinned = ts;
        }
    });
}

/// Forwards a message to another chat with a comment.
pub async fn forward(c: Ctx, m: Message, to: ChatId, text: String) -> Result<(), String> {
    let me = c.snap.with_untracked(|s| s.me.clone());
    let author_name = c
        .snap
        .with_untracked(|s| s.user(&m.author).map(|u| u.display_name.clone()))
        .unwrap_or_default();
    let (ts, seq_no) = if c.live {
        let sent = bridge::forward_message(&m, &to.0, &text).await?;
        (sent.ts, sent.seq_no)
    } else {
        (now_mcs(), 0)
    };
    // A reply is forwarded as itself, not as the message it answers.
    let original = if m.forwarded.is_empty() || m.reply_to().is_some() {
        vec![Forwarded {
            author: m.author.clone(),
            author_name,
            ts: m.ts,
            chat_id: Some(m.chat_id.clone()),
            text: m.text.clone(),
            attachments: m.attachments.clone(),
            quote: None,
            reply: false,
        }]
    } else {
        m.forwarded.clone()
    };
    let message = Message {
        chat_id: to,
        ts,
        author: me,
        text: text.trim().to_owned(),
        thread_parent: None,
        edited: false,
        important: false,
        reactions: vec![],
        thread: None,
        seq_no,
        attachments: vec![],
        forwarded: original,
        call: None,
    };
    upsert(c, message, None);
    Ok(())
}

/// A link from a message: in the app it opens in the default browser, in the demo in a new tab.
pub fn open_url(c: Ctx, url: String) {
    if let Some((chat, ts)) = parse_message_link(&url)
        && c.snap.with_untracked(|s| s.chat(&chat).is_some())
    {
        return jump_to_message(c, chat, ts, None);
    }
    if !c.live {
        let _ = window().open_with_url_and_target(&url, "_blank");
        return;
    }
    spawn_local(async move {
        if let Err(e) = bridge::open_url(&url).await {
            report(c, e);
        }
    });
}

pub fn apply_boot(c: Ctx, boot: Result<bridge::Boot, String>) {
    match boot {
        Ok(bridge::Boot::Ready { snapshot }) => {
            let first = snapshot.sections.iter().flat_map(|s| s.chats.first()).next().cloned();
            c.snap.set(snapshot);
            c.phase.set(Phase::Ready);
            c.online.set(true);
            refresh_presence(c);
            load_threads(c);
            if let Some(id) = first {
                open_chat(c, id);
            }
        }
        Ok(bridge::Boot::NeedLogin { reason }) => c.phase.set(Phase::Login { reason }),
        Err(e) => c.phase.set(Phase::Login { reason: Some(e) }),
    }
}

/// Subscriptions to backend events.
pub fn subscribe(c: Ctx) {
    spawn_local(async move {
        bridge::listen(bridge::EVENT, move |ev: LiveEvent| handle_event(c, ev)).await;
        bridge::listen(bridge::OPEN_EVENT, move |t: bridge::OpenTarget| {
            jump_to_message(c, t.chat_id, t.ts, t.thread_parent);
        })
        .await;
        bridge::listen(bridge::FILES_EVENT, move |files: Vec<bridge::PendingFile>| {
            c.pending_files.update(|p| {
                for f in files {
                    if !p.iter().any(|x| x.path == f.path) {
                        p.push(f);
                    }
                }
            });
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
            Err(e) => report(c, format!("{}: {e}", tr().err_profile)),
        }
    });
}

/// Leaves the channel, removes it from the sidebar and opens the first remaining chat.
pub fn leave_chat(c: Ctx, id: ChatId) {
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::leave_chat(&id.0).await
        {
            return report(c, format!("{}: {e}", tr().err_leave));
        }
        c.snap.update(|s| {
            s.chats.retain(|ch| ch.id != id);
            s.messages.retain(|m| m.chat_id != id);
            for sec in &mut s.sections {
                sec.chats.retain(|x| x != &id);
            }
        });
        if c.current.get_untracked() == id {
            let next = c
                .snap
                .with_untracked(|s| s.sections.iter().flat_map(|sec| sec.chats.first()).next().cloned());
            if let Some(n) = next {
                open_chat(c, n);
            }
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
            Err(e) => report(c, format!("{}: {e}", tr().err_open_dm)),
        }
    });
}

/// How long after a pushed `Heartbeat` a person stays online, whatever a poll says
/// (the same window as `ONLINE_WINDOW_MCS` in `mostik-yandex`).
const ONLINE_WINDOW_MS: f64 = 10.0 * 60.0 * 1000.0;

thread_local! {
    /// When the last `Heartbeat` push of each person arrived (local time, ms).
    static LAST_HEARTBEAT: std::cell::RefCell<std::collections::HashMap<UserId, f64>> =
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
/// at most once a minute.
pub fn refresh_me(c: Ctx) {
    thread_local! {
        static LAST: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    }
    let now = js_sys::Date::now();
    if !c.live || c.phase.get_untracked() != Phase::Ready || now - LAST.get() < 60_000.0 {
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

/// Own availability and status.
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
        )
    };
    if !c.live {
        return apply(c);
    }
    spawn_local(async move {
        match bridge::set_status(availability.code(), &emoji, &text, duration).await {
            Ok(()) => apply(c),
            Err(e) => report(c, format!("{}: {e}", tr().err_status)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::split_message_link;

    #[test]
    fn splits_message_links() {
        let own = "https://messenger.360.yandex.ru/chats/0%2F0%2Fabc/123";
        assert_eq!(split_message_link(own), Some(("0%2F0%2Fabc", 123)));
        assert_eq!(
            split_message_link("https://telemost.360.yandex.ru/chat/#/chats/a_b/7/"),
            Some(("a_b", 7))
        );
        assert_eq!(
            split_message_link("https://messenger.360.yandex.ru/#/chats/x/9"),
            Some(("x", 9))
        );
        assert_eq!(
            split_message_link("https://messenger.360.yandex.ru/chats/x/notanumber"),
            None
        );
        assert_eq!(split_message_link("https://example.com/chats/x/1"), None);
    }
}
