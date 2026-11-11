//! UI operations. In live mode they go through the Tauri backend, in demo mode they stay local.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::{Ctx, Phase};
use crate::bridge::{self, LiveEvent};
use crate::i18n::tr;

const PAGE: usize = 50;

fn report(c: Ctx, e: String) {
    leptos::logging::warn!("{e}");
    c.error.set(Some(e));
}

fn now_mcs() -> Timestamp {
    (js_sys::Date::now() * 1000.0) as Timestamp
}

/// Inserts or replaces a message; keeps the thread and unread counters up to date.
pub fn upsert(c: Ctx, m: Message, author: Option<User>) {
    let current = c.current.get_untracked();
    let open_thread = c.thread.get_untracked();
    c.snap.update(|s| {
        if let Some(a) = author
            && !s.users.iter().any(|u| u.id == a.id)
        {
            s.users.push(a);
        }
        if let Some(old) = s.messages.iter_mut().find(|x| x.chat_id == m.chat_id && x.ts == m.ts) {
            // An edit push may come without reactions or a sequence number: keep the known ones.
            // Reaction changes arrive as separate `Reactions` events.
            let thread = m.thread.clone().or_else(|| old.thread.clone());
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
            let viewing = current == m.chat_id
                && document_visible()
                && c.hold_unread.with_untracked(|h| h.as_ref() != Some(&m.chat_id));
            if !mine && !viewing {
                ch.unread += 1;
                if s.users
                    .iter()
                    .find(|u| u.id == s.me)
                    .is_some_and(|me| m.text.contains(&format!("@{}", me.display_name)))
                {
                    ch.mentions += 1;
                }
            }
        }
        s.messages.push(m.clone());
        sort_sections(s);
    });
    let viewing_reply = m.thread_parent.is_some() && open_thread == m.thread_parent;
    let read_on_open = c.settings.with_untracked(|s| s.mark_read_on_open);
    let held = c.hold_unread.with_untracked(|h| h.as_ref() == Some(&m.chat_id));
    if c.live
        && read_on_open
        && !held
        && m.chat_id == current
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

pub fn remove(c: Ctx, chat: &ChatId, ts: Timestamp) {
    c.snap.update(|s| {
        let Some(pos) = s.messages.iter().position(|m| &m.chat_id == chat && m.ts == ts) else {
            return;
        };
        let removed = s.messages.remove(pos);
        if let Some(parent) = removed.thread_parent
            && let Some(p) = s.messages.iter_mut().find(|x| &x.chat_id == chat && x.ts == parent)
            && let Some(t) = p.thread.as_mut()
        {
            t.reply_count = t.reply_count.saturating_sub(1);
        }
    });
}

pub fn handle_event(c: Ctx, ev: LiveEvent) {
    match ev {
        LiveEvent::Upsert { message, author } => upsert(c, *message, author),
        LiveEvent::Deleted { chat_id, ts } => remove(c, &chat_id, ts),
        LiveEvent::Reactions { chat_id, ts, reactions } => c.snap.update(|s| {
            if let Some(m) = s.messages.iter_mut().find(|m| m.chat_id == chat_id && m.ts == ts) {
                m.reactions = reactions;
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
        LiveEvent::Online { user, online } => c.snap.update(|s| {
            if let Some(u) = s.users.iter_mut().find(|u| u.id == user) {
                u.presence = if online { Presence::Online } else { Presence::Offline };
            }
        }),
        LiveEvent::Status(p) => apply_presence(c, vec![p]),
        LiveEvent::Pinned { chat_id, ts } => set_pinned(c, &chat_id, ts),
        LiveEvent::Connection { online } => c.online.set(online),
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
    if c.view.get_untracked() == crate::app::ViewMode::Drafts {
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
        Ok(messages) => {
            let full = messages.len() < PAGE;
            c.snap.update(|s| {
                s.messages.retain(|m| &m.chat_id != id || m.thread_parent.is_some());
                s.messages.extend(messages);
            });
            c.exhausted.update(|e| {
                if full {
                    e.insert(id.clone());
                } else {
                    e.remove(id);
                }
            });
            if read_on_open {
                mark_read(c, id.clone());
            }
        }
        Err(e) => report(c, format!("{}: {e}", tr().err_history)),
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
    let same = c.current.get_untracked() == chat;
    let load = if same {
        if c.view.get_untracked() == crate::app::ViewMode::Drafts {
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
                Ok(messages) => {
                    if messages.len() < PAGE {
                        c.exhausted.update(|e| {
                            e.insert(chat2.clone());
                        });
                    }
                    c.snap.update(|s| {
                        for m in messages {
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
            Ok(messages) => {
                if messages.len() < PAGE {
                    c.exhausted.update(|e| {
                        e.insert(id.clone());
                    });
                }
                c.snap.update(|s| {
                    for m in messages {
                        if !s.messages.iter().any(|x| x.chat_id == m.chat_id && x.ts == m.ts) {
                            s.messages.push(m);
                        }
                    }
                });
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
    if !c.live {
        return;
    }
    let id = c.current.get_untracked();
    spawn_local(async move {
        match bridge::history(&id.0, None, Some(parent)).await {
            Ok(replies) => c.snap.update(|s| {
                s.messages
                    .retain(|m| !(m.chat_id == id && m.thread_parent == Some(parent)));
                s.messages.extend(replies);
            }),
            Err(e) => report(c, format!("{}: {e}", tr().err_thread)),
        }
    });
}

pub async fn send(c: Ctx, chat: ChatId, text: String, thread: Option<Timestamp>) -> Result<(), String> {
    let me = c.snap.with_untracked(|s| s.me.clone());
    let message = if c.live {
        bridge::send_message(&chat.0, &text, thread).await?
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
        }
    };
    upsert(c, message, None);
    Ok(())
}

/// Sends the picked files with a caption; not available in demo mode.
pub async fn send_files(c: Ctx, chat: ChatId, text: String, thread: Option<Timestamp>) -> Result<(), String> {
    if !c.live {
        return Err(tr().only_in_app_files.into());
    }
    let paths: Vec<String> = c.pending_files.get_untracked().into_iter().map(|f| f.path).collect();
    let sent = bridge::send_files(&chat.0, thread, paths, &text).await?;
    c.pending_files.set(vec![]);
    for m in sent {
        upsert(c, m, None);
    }
    Ok(())
}

pub async fn edit(c: Ctx, m: Message, text: String) -> Result<(), String> {
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
        remove(c, &m.chat_id, m.ts);
    });
}

/// Adds reaction `kind`, or removes it if it is already mine.
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
            let Some(msg) = s.messages.iter_mut().find(|x| x.chat_id == m.chat_id && x.ts == m.ts) else {
                return;
            };
            match msg.reactions.iter_mut().find(|r| r.kind == kind) {
                Some(r) if add && !r.mine => {
                    r.count += 1;
                    r.mine = true;
                }
                Some(r) if !add && r.mine => {
                    r.count = r.count.saturating_sub(1);
                    r.mine = false;
                }
                Some(_) => {}
                None if add => msg.reactions.push(Reaction {
                    kind,
                    emoji: reactions::emoji(kind).to_owned(),
                    count: 1,
                    mine: true,
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

/// A call in a chat, like a Slack huddle: the chat meeting opens in the browser.
pub fn start_call(c: Ctx, chat: ChatId) {
    if !c.live {
        return report(c, tr().only_in_app_calls.into());
    }
    c.notice.set(Some(tr().call_opening.into()));
    spawn_local(async move {
        if let Err(e) = bridge::start_call(&chat.0).await {
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

/// "Mute" / "Unmute" for a chat.
pub fn toggle_mute(c: Ctx, id: ChatId) {
    let muted = !c.snap.with_untracked(|s| s.chat(&id).is_some_and(|ch| ch.muted));
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::set_muted(&id.0, muted).await
        {
            return report(c, format!("{}: {e}", tr().err_settings));
        }
        c.snap.update(|s| {
            if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == id) {
                ch.muted = muted;
            }
        });
        c.notice
            .set(Some(if muted { tr().muted_notice } else { tr().unmuted_notice }.into()));
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
        let target = match ch.kind {
            ChatKind::Direct { .. } | ChatKind::Notes => "dms",
            _ if id.0.starts_with("0/22/") || id.0.starts_with("0/32/") => "meetings",
            _ => "channels",
        };
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
    let original = if m.forwarded.is_empty() {
        vec![Forwarded {
            author: m.author.clone(),
            author_name,
            ts: m.ts,
            chat_id: Some(m.chat_id.clone()),
            text: m.text.clone(),
            attachments: m.attachments.clone(),
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
        bridge::listen(bridge::SOUND_EVENT, move |_: serde_json::Value| {
            crate::sound::play(&c.settings.with_untracked(|s| s.sound.clone()));
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
        bridge::listen(bridge::SIGNED_OUT_EVENT, move |reason: String| {
            c.phase.set(Phase::Login { reason: Some(reason) });
        })
        .await;
        bridge::listen(bridge::SNAPSHOT_EVENT, move |snapshot: Snapshot| {
            let current = c.current.get_untracked();
            c.snap.set(snapshot);
            open_chat(c, current);
        })
        .await;
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

/// Applies presence and statuses to the people of the snapshot. We always count ourselves as online:
/// while Mostik runs, it sends `Heartbeat`.
pub fn apply_presence(c: Ctx, list: Vec<PresenceInfo>) {
    c.snap.update(|s| {
        let me = s.me.clone();
        for p in list {
            if let Some(u) = s.users.iter_mut().find(|u| u.id == p.user) {
                if let Some(online) = p.online {
                    u.presence = if online { Presence::Online } else { Presence::Offline };
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
