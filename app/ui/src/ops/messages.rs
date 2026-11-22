//! Actions on messages: sending with mentions and replies, editing, deleting, reactions,
//! marks, typing, "Mark unread", pins and forwarding.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::Ctx;
use crate::bridge;
use crate::i18n::tr;

use super::snapshot::{remove, upsert};
use super::{failed, now_mcs};

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
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
            media: None,
            reply: true,
        }
    }
}

/// Starts a reply to `msg` (quoting `quote`, when given) in the composer of its chat or thread.
pub fn start_reply(c: Ctx, msg: Message, quote: Option<String>) {
    c.editing.set(None);
    let quote = quote.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty());
    let key = crate::app::drafts::DraftKey::field(msg.chat_id.clone(), msg.thread_parent);
    crate::app::drafts::set_reply(c, &key, Some(ReplyTarget { msg, quote }));
}

/// Sends a text message, with `reply` a reply; a text longer than Telemost takes goes as several
/// messages, the reply with the first of them.
pub async fn send(
    c: Ctx,
    chat: ChatId,
    text: String,
    thread: Option<Timestamp>,
    reply: Option<ReplyTarget>,
    options: bridge::SendOptions,
) -> Result<(), String> {
    let text = with_mention_ids(c, &text);
    let me = c.snap.with_untracked(|s| s.me.clone());
    let reply_ref = reply.as_ref().map(|r| bridge::ReplyRef {
        ts: r.msg.ts,
        quote: r.quote.clone(),
    });
    let mut unsent = None;
    let mut messages = if c.live {
        let sent = bridge::send_message(&chat.0, &text, thread, reply_ref, options).await?;
        unsent = sent.unsent;
        sent.messages
    } else {
        vec![Message {
            chat_id: chat.clone(),
            ts: now_mcs(),
            author: me,
            text: text.clone(),
            thread_parent: thread,
            edited: false,
            important: options.important,
            reactions: vec![],
            thread: None,
            seq_no: 0,
            attachments: vec![],
            forwarded: vec![],
            call: None,
            system: None,
            media: None,
            reads: None,
        }]
    };
    if let (Some(r), Some(first)) = (&reply, messages.first_mut()) {
        first.forwarded = vec![r.forwarded(c)];
    }
    for message in messages {
        upsert(c, message, None);
    }
    match unsent {
        Some(rest) => {
            keep_unsent(c, &chat, thread, &text, reply.as_ref(), &rest.text);
            Err(tr().partly_sent(&super::in_own_words(&rest.why)))
        }
        None => Ok(()),
    }
}

/// After a long text went only in part, its field keeps only the parts that did not go, and not the
/// reply the first part carried: sending again does not repeat what went. Not if the user has
/// changed the text meanwhile; a reply they picked meanwhile stays.
fn keep_unsent(c: Ctx, chat: &ChatId, thread: Option<Timestamp>, sent: &str, reply: Option<&ReplyTarget>, rest: &str) {
    let key = crate::app::drafts::DraftKey::field(chat.clone(), thread);
    let draft = crate::app::drafts::get(c, &key);
    if with_mention_ids(c, draft.text.trim()) != sent {
        return;
    }
    let (rest, named) = c.snap.with_untracked(|s| {
        s.mention_names()
            .with(|who| mostik_core::markup::ids_to_names(rest, who))
    });
    c.picked.update(|p| {
        for (name, id) in named {
            p.insert(name, UserId(id));
        }
    });
    crate::app::drafts::set_text(c, &key, &rest, None);
    if reply.is_some() && draft.reply.as_ref() == reply {
        crate::app::drafts::set_reply(c, &key, None);
    }
}

/// Sends the files attached in `chat` (in the thread of `thread`) with a caption; not available in
/// demo mode.
/// A message with files on its way, shown in the feed at once, as the official client does: each
/// file with how far its upload has got, and a click on it cancels it. On an error it stays, "not
/// sent", with a retry. The sent messages take its place.
#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    /// The id of the sending for the backend: its progress and cancels name it.
    pub id: String,
    pub chat: ChatId,
    pub thread: Option<Timestamp>,
    pub text: String,
    pub files: Vec<OutFile>,
    /// Why it was not sent.
    pub failed: Option<String>,
    /// When it was sent, for its place among the messages.
    pub ts: Timestamp,
    /// How it goes: without a sound, as important.
    pub options: bridge::SendOptions,
}

/// A file of an [`Outgoing`] message.
#[derive(Clone, Debug, PartialEq)]
pub struct OutFile {
    pub file: bridge::PendingFile,
    /// Bytes uploaded so far.
    pub sent: u64,
    pub cancelled: bool,
}

impl OutFile {
    /// Uploaded whole: too late to cancel.
    pub fn done(&self) -> bool {
        self.file.size > 0 && self.sent >= self.file.size
    }
}

/// Sends the files attached to the field of `chat` (or of its thread of `thread`) with `text` as
/// an [`Outgoing`] message: the field gives them up at once and the feed shows the message while
/// the files go.
pub fn send_files(
    c: Ctx,
    chat: ChatId,
    text: String,
    thread: Option<Timestamp>,
    options: bridge::SendOptions,
) -> Result<(), String> {
    if !c.live {
        return Err(tr().only_in_app_files.into());
    }
    let files = attached(c, &chat, thread);
    c.pending_files.update(|all| {
        all.remove(&(chat.clone(), thread));
    });
    let id = format!("up-{:x}", super::random_u64());
    c.outbox.update(|o| {
        o.push(Outgoing {
            id: id.clone(),
            chat,
            thread,
            text,
            files: files
                .into_iter()
                .map(|file| OutFile {
                    file,
                    sent: 0,
                    cancelled: false,
                })
                .collect(),
            failed: None,
            ts: super::now_mcs(),
            options,
        })
    });
    run_upload(c, id);
    Ok(())
}

/// Uploads and sends [`Outgoing`] message `id`: its files that are not cancelled, with its text.
fn run_upload(c: Ctx, id: String) {
    let Some(out) = c.outbox.with_untracked(|o| o.iter().find(|m| m.id == id).cloned()) else {
        return;
    };
    let paths: Vec<String> = out
        .files
        .iter()
        .filter(|f| !f.cancelled)
        .map(|f| f.file.path.clone())
        .collect();
    let text = with_mention_ids(c, &out.text);
    spawn_local(async move {
        match bridge::send_files(&out.chat.0, out.thread, paths, &text, Some(&id), out.options).await {
            Ok(sent) => {
                // A file cancelled before a retry was not asked for again: the pasted bytes the
                // app kept for it go here.
                let cancelled: Vec<String> = c.outbox.with_untracked(|o| {
                    o.iter()
                        .filter(|m| m.id == id)
                        .flat_map(|m| m.files.iter().filter(|f| f.cancelled).map(|f| f.file.path.clone()))
                        .collect()
                });
                c.outbox.update(|o| o.retain(|m| m.id != id));
                for m in sent {
                    upsert(c, m, None);
                }
                for path in cancelled {
                    if let Err(e) = bridge::forget_file(&path).await {
                        leptos::logging::warn!("forget_file: {e}");
                    }
                }
            }
            Err(e) => c.outbox.update(|o| {
                if let Some(m) = o.iter_mut().find(|m| m.id == id) {
                    m.failed = Some(e);
                }
            }),
        }
    });
}

/// "Retry" on a message that was not sent: what has not gone goes again, its files from the start.
pub fn retry_upload(c: Ctx, id: &str) {
    c.outbox.update(|o| {
        if let Some(m) = o.iter_mut().find(|m| m.id == id) {
            m.failed = None;
            for f in &mut m.files {
                f.sent = 0;
            }
        }
    });
    run_upload(c, id.to_owned());
}

/// "Remove" on a message that was not sent: it goes, and so do the pasted files it kept.
pub fn drop_upload(c: Ctx, id: &str) {
    let gone = c.outbox.with_untracked(|o| o.iter().find(|m| m.id == id).cloned());
    c.outbox.update(|o| o.retain(|m| m.id != id));
    for f in gone.into_iter().flat_map(|m| m.files) {
        spawn_local(async move {
            if let Err(e) = bridge::forget_file(&f.file.path).await {
                leptos::logging::warn!("forget_file: {e}");
            }
        });
    }
}

/// A click on the upload circle of a file: the message goes without it.
pub fn cancel_file(c: Ctx, id: &str, path: &str) {
    c.outbox.update(|o| {
        if let Some(f) = o
            .iter_mut()
            .find(|m| m.id == id)
            .and_then(|m| m.files.iter_mut().find(|f| f.file.path == path))
        {
            f.cancelled = true;
        }
    });
    let (id, path) = (id.to_owned(), path.to_owned());
    spawn_local(async move {
        if let Err(e) = bridge::cancel_upload(&id, &path).await {
            leptos::logging::warn!("cancel_upload: {e}");
        }
    });
}

/// How far a file of an [`Outgoing`] message has got.
pub fn upload_progress(c: Ctx, p: bridge::UploadProgress) {
    c.outbox.update(|o| {
        if let Some(f) = o
            .iter_mut()
            .find(|m| m.id == p.upload)
            .and_then(|m| m.files.iter_mut().find(|f| f.file.path == p.path))
        {
            f.sent = p.sent;
            f.file.size = p.total;
        }
    });
}

/// Attaches `files` to the message being written in `chat`, or in its thread of `thread`; a file
/// already there is not added again.
pub fn attach_files(c: Ctx, chat: ChatId, thread: Option<Timestamp>, files: Vec<bridge::PendingFile>) {
    if files.is_empty() {
        return;
    }
    c.pending_files.update(|all| {
        let list = all.entry((chat, thread)).or_default();
        for f in files {
            if !list.iter().any(|x| x.path == f.path) {
                list.push(f);
            }
        }
    });
}

/// Takes the file at `path` off the message being written in `chat` (in the thread of `thread`);
/// a pasted file is no longer kept in memory.
pub fn detach_file(c: Ctx, chat: &ChatId, thread: Option<Timestamp>, path: &str) {
    let key = (chat.clone(), thread);
    c.pending_files.update(|all| {
        if let Some(list) = all.get_mut(&key) {
            list.retain(|f| f.path != path);
            if list.is_empty() {
                all.remove(&key);
            }
        }
    });
    forget_pasted(c, path.to_owned());
}

/// The bytes of a pasted file at `path`, kept in memory for its sending, are not needed any more.
fn forget_pasted(c: Ctx, path: String) {
    if c.live {
        spawn_local(async move {
            if let Err(e) = bridge::forget_file(&path).await {
                leptos::logging::warn!("forget_file: {e}");
            }
        });
    }
}

/// Files attached to the message being written in `chat` (in the thread of `thread`), untracked.
pub fn attached(c: Ctx, chat: &ChatId, thread: Option<Timestamp>) -> Vec<bridge::PendingFile> {
    c.pending_files
        .with_untracked(|all| all.get(&(chat.clone(), thread)).cloned().unwrap_or_default())
}

/// Replaces the text of `m`. The new text goes over the message as it is now: `m` is the copy
/// taken when the editor opened, and its reactions and marks may have changed since.
pub async fn edit(c: Ctx, m: Message, text: String) -> Result<(), String> {
    let text = with_mention_ids(c, &text);
    if c.live {
        bridge::edit_message(&m, &text).await?;
    }
    let now = c.snap.with_untracked(|s| {
        s.messages
            .iter()
            .find(|x| x.chat_id == m.chat_id && x.ts == m.ts)
            .cloned()
    });
    // Gone meanwhile (deleted, or the chat was left): nothing to show it in.
    if let Some(now) = now {
        upsert(
            c,
            Message {
                text,
                edited: true,
                ..now
            },
            None,
        );
    }
    Ok(())
}

pub fn delete(c: Ctx, m: Message) {
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::delete_message(&m).await
        {
            return failed(c, tr().err_delete, e);
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
            return failed(c, tr().err_reaction, e);
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
            return failed(c, tr().err_important, e);
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

/// "Mark unread" from this message, as in Slack.
pub fn mark_unread(c: Ctx, m: &Message) {
    let m = m.clone();
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::mark_unread(&m.chat_id.0, m.ts, m.seq_no).await
        {
            return failed(c, tr().err_settings, e);
        }
        c.snap.update(|s| {
            let n = s
                .messages
                .iter()
                .filter(|x| x.chat_id == m.chat_id && x.thread_parent.is_none() && x.ts >= m.ts)
                .count() as u32;
            if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == m.chat_id) {
                ch.unread = n.max(1);
                // The read position goes back to just before it, as Telemost's does.
                if m.seq_no > 0 {
                    ch.seen_seq = Some(m.seq_no - 1);
                }
                // Every message from it on is counted now.
                ch.uncounted.clear();
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
            return failed(c, tr().err_pin, e);
        }
        set_pinned(c, &m.chat_id, ts);
    });
}

pub(super) fn set_pinned(c: Ctx, chat: &ChatId, ts: Option<Timestamp>) {
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
            media: None,
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
        system: None,
        media: None,
        reads: None,
    };
    upsert(c, message, None);
    Ok(())
}
