//! Mapping of protocol responses to the `mostik-core` domain model.
//!
//! Parsing tolerates missing fields: the protocol is undocumented.
//! The `ChatId` patterns were confirmed on a live session.

use std::collections::BTreeMap;

use mostik_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(*k))?.as_str()
}

fn n(v: &Value, path: &[&str]) -> Option<u64> {
    let x = path.iter().try_fold(v, |v, k| v.get(*k))?;
    x.as_u64().or_else(|| x.as_f64().map(|f| f as u64))
}

/// Telemost meeting chat: namespaces 22 and 32 (`namespace_aliases.telemost` in config.json).
pub fn is_meeting_chat(chat_id: &str) -> bool {
    chat_id.starts_with("0/22/") || chat_id.starts_with("0/32/")
}

/// Thread chat for message `ts` of chat `chat_id`: `0/<ns>/<uuid>` → `100/<ns>/<uuid>_<ts>`.
/// Verified on a live session for `0/0/…` groups; other chats do not support threads.
pub fn thread_chat_id(chat_id: &str, ts: Timestamp) -> Option<String> {
    let rest = chat_id.strip_prefix("0/")?;
    let (ns, uuid) = rest.split_once('/')?;
    (!uuid.is_empty() && !uuid.contains('/')).then(|| format!("100/{ns}/{uuid}_{ts}"))
}

/// Inverse of [`thread_chat_id`]: `(parent chat, time of the parent message)`.
pub fn thread_parent(thread_id: &str) -> Option<(String, Timestamp)> {
    let rest = thread_id.strip_prefix("100/")?;
    let (ns, tail) = rest.split_once('/')?;
    let (uuid, ts) = tail.rsplit_once('_')?;
    Some((format!("0/{ns}/{uuid}"), ts.parse().ok()?))
}

/// Chat kind by its ID: `<guid>_<guid>` is direct, `1/…` is a channel, everything else is a group.
pub fn chat_kind(chat_id: &str, me: Option<&UserId>, is_public: bool) -> ChatKind {
    if !chat_id.contains('/')
        && let Some((a, b)) = chat_id.split_once('_')
    {
        let me = me.map(|u| u.0.as_str());
        if a == b && Some(a) == me {
            return ChatKind::Notes;
        }
        let peer = if Some(a) == me { b } else { a };
        return ChatKind::Direct {
            peer: UserId(peer.to_owned()),
        };
    }
    if chat_id.starts_with("1/") {
        return ChatKind::Broadcast;
    }
    ChatKind::Group { public: is_public }
}

fn reactions(sm: &Value, me: Option<&UserId>) -> Vec<Reaction> {
    // `(type, who)` of the latest reactions.
    let recent: Vec<(u64, &str)> = sm
        .get("RecentUserReactions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| Some((n(r, &["Type"])?, s(r, &["UserInfo", "Guid"])?)))
        .collect();
    let mine: Vec<u64> = recent
        .iter()
        .filter(|(_, who)| me.is_some_and(|me| me.0 == *who))
        .map(|(kind, _)| *kind)
        .collect();
    sm.get("Reactions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let kind = n(r, &["Type"])?;
            let count = n(r, &["Count"]).unwrap_or(0) as u32;
            (count > 0).then(|| Reaction {
                kind,
                emoji: crate::reactions::emoji(kind).to_owned(),
                count,
                mine: mine.contains(&kind),
                users: recent
                    .iter()
                    .filter(|(k, _)| *k == kind)
                    .map(|(_, who)| UserId((*who).to_owned()))
                    .collect(),
            })
        })
        .collect()
}

/// People from `RecentUserReactions`, with the names the UI shows in the reaction tooltip.
pub fn reaction_authors(sm: &Value) -> Vec<User> {
    sm.get("RecentUserReactions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| r.get("UserInfo").and_then(user_from))
        .collect()
}

/// People the message mentions (`MentionedUsers`, which Telemost adds next to `ClientMessage`),
/// so `@<guid>` in the text can show their names.
pub fn mentioned_users(sm: &Value) -> Vec<User> {
    sm.get("MentionedUsers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(user_from)
        .collect()
}

/// `Plain` fields that a deleted message still has.
const PLAIN_SERVICE_KEYS: &[&str] = &["ChatId", "Timestamp", "PayloadId"];

/// `Plain` without content: this is how the web client deletes a message.
fn is_deletion(plain: &Value) -> bool {
    plain
        .as_object()
        .is_some_and(|o| o.keys().all(|k| PLAIN_SERVICE_KEYS.contains(&k.as_str())))
}

fn attachment(kind: AttachmentKind, v: &Value) -> Option<Attachment> {
    let info = v.get("FileInfo")?;
    let file_id = s(info, &["Id2"]).or_else(|| s(info, &["Id"]))?.to_owned();
    Some(Attachment {
        kind,
        file_id,
        name: s(info, &["Name"]).unwrap_or("file").to_owned(),
        size: n(info, &["Size"]).unwrap_or(0),
        width: n(v, &["Width"]).unwrap_or(0) as u32,
        height: n(v, &["Height"]).unwrap_or(0) as u32,
    })
}

/// Attachments of a `Plain`: gallery images, a single image, a file.
fn attachments(plain: &Value) -> Vec<Attachment> {
    let mut out: Vec<Attachment> = plain
        .pointer("/Gallery/Items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| attachment(AttachmentKind::Image, item.get("Image")?))
        .collect();
    if let Some(img) = plain.get("Image").and_then(|i| attachment(AttachmentKind::Image, i)) {
        out.push(img);
    }
    if let Some(file) = plain.get("MiscFile").and_then(|f| attachment(AttachmentKind::File, f)) {
        out.push(file);
    }
    out
}

/// Feed text of a `Plain`. For images and files it is the caption (may be empty).
fn plain_text(plain: &Value) -> String {
    if let Some(t) = s(plain, &["Text", "MessageText"]) {
        return t.to_owned();
    }
    if let Some(t) = s(plain, &["Gallery", "Text"]) {
        return t.to_owned();
    }
    if plain.get("Gallery").is_some() || plain.get("Image").is_some() || plain.get("MiscFile").is_some() {
        return String::new();
    }
    if plain.get("Sticker").is_some() {
        return "Sticker".to_owned();
    }
    if plain.get("Voice").is_some() {
        return "🎤 Voice message".to_owned();
    }
    "Message (this type is not supported yet)".to_owned()
}

fn user_from(v: &Value) -> Option<User> {
    let guid = s(v, &["Guid"])?;
    let name = s(v, &["DisplayName"]).or_else(|| s(v, &["PublicName"])).unwrap_or(guid);
    Some(User {
        id: UserId(guid.to_owned()),
        display_name: name.to_owned(),
        title: None,
        presence: Presence::Offline,
        status: None,
        availability: Default::default(),
        is_robot: v.get("IsRobot").and_then(Value::as_bool).unwrap_or(false),
        avatar: s(v, &["AvatarId"]).filter(|a| !a.is_empty()).map(str::to_owned),
    })
}

/// Message from a `ServerMessage` (history) or a push body: both have the same structure.
pub enum Parsed {
    Message(Box<Message>),
    Deleted {
        chat_id: ChatId,
        ts: Timestamp,
        /// For a thread reply, the time of the parent message.
        thread_parent: Option<Timestamp>,
    },
    /// A service message or a system one the feed does not show.
    Skip,
}

fn parse_server_message(sm: &Value, chat_hint: Option<&str>, me: Option<&UserId>) -> Parsed {
    let info = &sm["ServerMessageInfo"];
    let Some(plain) = sm.pointer("/ClientMessage/Plain") else {
        return call_note(sm).map_or(Parsed::Skip, |m| Parsed::Message(Box::new(m)));
    };
    let Some(chat) = s(plain, &["ChatId"]).or(chat_hint) else {
        return Parsed::Skip;
    };
    let Some(ts) = n(info, &["Timestamp"]).or_else(|| n(plain, &["Timestamp"])) else {
        return Parsed::Skip;
    };
    // A thread reply comes from the thread chat: attach it to the parent message.
    let (chat_id, parent) = match thread_parent(chat) {
        Some((p, pts)) => (ChatId(p), Some(pts)),
        None => (ChatId(chat.to_owned()), None),
    };
    let forwarded = forwarded(sm);
    if (is_deletion(plain) && forwarded.is_empty()) || info.get("Deleted").and_then(Value::as_bool) == Some(true) {
        return Parsed::Deleted {
            chat_id,
            ts,
            thread_parent: parent,
        };
    }
    let thread = n(info, &["ThreadState", "LastSeqNo"])
        .filter(|c| *c > 0)
        .map(|count| ThreadSummary {
            reply_count: count as u32,
            last_reply_ts: n(info, &["ThreadState", "LastTsMcs"]).unwrap_or(ts),
            participants: vec![],
        });
    Parsed::Message(Box::new(Message {
        chat_id,
        ts,
        author: UserId(s(info, &["From", "Guid"]).unwrap_or_default().to_owned()),
        text: if forwarded.is_empty() || plain.get("Text").is_some() {
            plain_text(plain)
        } else {
            String::new()
        },
        thread_parent: parent,
        edited: info.get("LastEditTimestamp").is_some(),
        important: plain.get("IsImportant").and_then(Value::as_bool).unwrap_or(false),
        reactions: reactions(sm, me),
        thread,
        seq_no: n(info, &["SeqNo"]).unwrap_or(0),
        attachments: attachments(plain),
        forwarded,
        call: None,
    }))
}

/// Telemost's note about a call in the chat:
/// - `SystemMessage{ChatId, MeetingStartedMessage | MeetingEndedMessage: {MeetingId}}` for a
///   group call, `PersonalMeetingEndedMessage: {MeetingId, CallType, Reason}` for a direct one;
///   the author is whoever started or ended the call;
/// - `MeetingCallingMessage{IncomingCall: {MeetingInfo: {ChatId, MeetingId, CreateTimestamp}}}`:
///   a direct call ringing. It repeats every 5 s while the call rings and history never has it;
///   all repeats become one note at the meeting's creation time, without a sequence number.
fn call_note(sm: &Value) -> Option<Message> {
    #[derive(Clone, Copy)]
    enum Kind {
        Started,
        Ringing,
        Ended,
        /// The end of a direct call, which always has a reason.
        PersonalEnded,
    }
    let info = &sm["ServerMessageInfo"];
    let (chat, ts, kind, meeting) = if let Some(sys) = sm.pointer("/ClientMessage/SystemMessage") {
        let (kind, meeting) = [
            ("MeetingStartedMessage", Kind::Started),
            ("PersonalMeetingStartedMessage", Kind::Started),
            ("MeetingEndedMessage", Kind::Ended),
            ("PersonalMeetingEndedMessage", Kind::PersonalEnded),
        ]
        .into_iter()
        .find_map(|(name, kind)| sys.get(name).map(|m| (kind, m)))?;
        (s(sys, &["ChatId"])?, n(info, &["Timestamp"])?, kind, meeting)
    } else {
        let meeting = sm.pointer("/ClientMessage/MeetingCallingMessage/IncomingCall/MeetingInfo")?;
        (
            s(meeting, &["ChatId"])?,
            n(meeting, &["CreateTimestamp"])?,
            Kind::Ringing,
            meeting,
        )
    };
    let url = meeting_url(s(meeting, &["MeetingId"])?);
    Some(Message {
        chat_id: ChatId(chat.to_owned()),
        ts,
        author: UserId(s(info, &["From", "Guid"]).unwrap_or_default().to_owned()),
        text: String::new(),
        thread_parent: None,
        edited: false,
        important: false,
        reactions: vec![],
        thread: None,
        seq_no: n(info, &["SeqNo"]).unwrap_or(0),
        attachments: vec![],
        forwarded: vec![],
        call: Some(match kind {
            Kind::Started => CallNote::Started { url },
            Kind::Ringing => CallNote::Ringing { url },
            Kind::Ended => CallNote::Ended { url, reason: None },
            // Zero is left out of the JSON: no `Reason` is 0, "declined" (the web client reads
            // it as `Reason || 0` too).
            Kind::PersonalEnded => CallNote::Ended {
                url,
                reason: CallEndReason::from_code(n(meeting, &["Reason"]).unwrap_or(0)),
            },
        }),
    })
}

/// A push about a direct call ringing for the user:
/// `MeetingCallingMessage{RingingId, IncomingCall | RingingEnded}`. The call itself (chat, link,
/// caller) comes from [`from_push`] as a `Ringing` note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ringing {
    /// The call rings; it repeats every 5 s.
    Incoming { id: String },
    /// It stopped ringing: the caller hung up, someone answered (maybe on another device), or
    /// nobody did in time.
    Ended { id: String },
}

pub fn ringing(body: &Value) -> Option<Ringing> {
    let sm = body.get("ServerMessage").unwrap_or(body);
    let calling = sm.pointer("/ClientMessage/MeetingCallingMessage")?;
    let id = s(calling, &["RingingId"])?.to_owned();
    if calling.get("IncomingCall").is_some() {
        Some(Ringing::Incoming { id })
    } else if calling.get("RingingEnded").is_some() {
        Some(Ringing::Ended { id })
    } else {
        None
    }
}

/// A meeting link from a `MeetingId`, which is either the link itself or the bare meeting number.
pub fn meeting_url(id: &str) -> String {
    if id.starts_with("https://") {
        id.to_owned()
    } else {
        format!("https://telemost.360.yandex.ru/j/{id}")
    }
}

/// Forwarded messages: `ForwardedMessages[]{Payload (like Plain), ServerMessageInfo{From, Timestamp}}`.
/// The quoted fragment of the `i`-th one is `Plain.ForwardedMessageStyles[i].Quote`. As in the web
/// client, a reply is the only forwarded message from the message's own chat (the thread chat for a
/// thread reply) or a quoted one.
fn forwarded(sm: &Value) -> Vec<Forwarded> {
    let list = sm
        .get("ForwardedMessages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let plain = sm.pointer("/ClientMessage/Plain");
    let own_chat = plain.and_then(|p| s(p, &["ChatId"]));
    list.iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let payload = f.get("Payload")?;
            let info = &f["ServerMessageInfo"];
            let chat = s(payload, &["ChatId"]);
            let quote = plain
                .and_then(|p| p.pointer(&format!("/ForwardedMessageStyles/{i}/Quote")))
                .and_then(Value::as_str)
                .filter(|q| !q.trim().is_empty())
                .map(str::to_owned);
            let gone = is_deletion(payload) || info.get("Deleted").and_then(Value::as_bool) == Some(true);
            Some(Forwarded {
                author: UserId(s(info, &["From", "Guid"]).unwrap_or_default().to_owned()),
                author_name: s(info, &["From", "DisplayName"]).unwrap_or_default().to_owned(),
                ts: n(info, &["Timestamp"])
                    .or_else(|| n(payload, &["Timestamp"]))
                    .unwrap_or(0),
                chat_id: chat.map(|c| ChatId(c.to_owned())),
                text: if gone { String::new() } else { plain_text(payload) },
                attachments: if gone { vec![] } else { attachments(payload) },
                reply: list.len() == 1 && (quote.is_some() || (chat.is_some() && chat == own_chat)),
                quote,
            })
        })
        .collect()
}

/// Parsed `history` response.
#[derive(Debug, Default)]
pub struct HistoryData {
    pub chats: Vec<Chat>,
    pub users: Vec<User>,
    pub messages: Vec<Message>,
    /// The last `(Timestamp, SeqNo)` of every chat, for read markers.
    pub last: BTreeMap<String, (Timestamp, u64)>,
}

/// Parses the response of the `history` WS method.
pub fn from_history(v: &Value, me: Option<&UserId>) -> HistoryData {
    let mut users: BTreeMap<String, User> = BTreeMap::new();
    let mut out = HistoryData::default();
    for chat in v.get("Chats").and_then(Value::as_array).into_iter().flatten() {
        let Some(id) = s(chat, &["ChatId"]) else { continue };
        let chat_id = ChatId(id.to_owned());
        // The peer of a direct chat, even when none of the chat's messages is theirs.
        if let Some(u) = chat.get("PartnerInfo").and_then(user_from) {
            users.entry(u.id.0.clone()).or_insert(u);
        }
        let mut last_ts = n(chat, &["LastTsMcs"]).unwrap_or(0);
        for item in chat.get("Messages").and_then(Value::as_array).into_iter().flatten() {
            let sm = item.get("ServerMessage").unwrap_or(item);
            if let Some(u) = sm.pointer("/ServerMessageInfo/From").and_then(user_from) {
                users.entry(u.id.0.clone()).or_insert(u);
            }
            for read in sm
                .get("RecentUserReads")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(u) = read.get("UserInfo").and_then(user_from) {
                    users.entry(u.id.0.clone()).or_insert(u);
                }
            }
            for u in reaction_authors(sm).into_iter().chain(mentioned_users(sm)) {
                users.entry(u.id.0.clone()).or_insert(u);
            }
            if let Parsed::Message(m) = parse_server_message(sm, Some(id), me) {
                last_ts = last_ts.max(m.ts);
                out.messages.push(*m);
            }
        }
        out.last
            .insert(id.to_owned(), (last_ts, n(chat, &["LastSeqNo"]).unwrap_or(0)));
        let info = &chat["ChatInfo"];
        let unread = n(chat, &["LastSeqNo"])
            .unwrap_or(0)
            .saturating_sub(n(chat, &["LastSeenByMeSeqNo"]).unwrap_or(0));
        out.chats.push(Chat {
            id: chat_id,
            kind: chat_kind(id, me, info.get("IsPublic").and_then(Value::as_bool).unwrap_or(false)),
            name: s(info, &["Name"]).unwrap_or_default().to_owned(),
            description: s(info, &["Description"]).filter(|d| !d.is_empty()).map(str::to_owned),
            member_count: n(info, &["MemberCount"]).unwrap_or(0) as u32,
            unread: unread.min(999) as u32,
            mentions: 0,
            muted: chat.get("Muted").and_then(Value::as_bool).unwrap_or(false),
            mute_mentions: false,
            last_ts,
            pinned: n(chat, &["PinnedMessageInfo", "PinnedMessageTs"]).filter(|ts| *ts > 0),
            organizations: info
                .get("OrganizationIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect(),
        });
    }
    out.users = users.into_values().collect();
    out
}

/// Found messages from the HTTP RPC `search` response with `entities: ["messages"]`:
/// `data.messages.items[].data` is a regular `ServerMessage`. Returns the messages and their authors.
pub fn search_messages(v: &Value, me: Option<&UserId>) -> (Vec<Message>, Vec<User>) {
    let mut users: BTreeMap<String, User> = BTreeMap::new();
    let mut out = Vec::new();
    for item in v
        .pointer("/data/messages/items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(sm) = item.get("data") else { continue };
        if let Some(u) = sm.pointer("/ServerMessageInfo/From").and_then(user_from) {
            users.entry(u.id.0.clone()).or_insert(u);
        }
        for u in reaction_authors(sm).into_iter().chain(mentioned_users(sm)) {
            users.entry(u.id.0.clone()).or_insert(u);
        }
        if let Parsed::Message(m) = parse_server_message(sm, None, me) {
            out.push(*m);
        }
    }
    (out, users.into_values().collect())
}

/// Followed threads from `history` with `Threads: true`: every item is a thread chat
/// (`100/<ns>/<uuid>_<ts>`) with its `ThreadParentMessage` and read counters. Returns the
/// threads, newest reply first, and the people they mention.
pub fn followed_threads(v: &Value, me: Option<&UserId>) -> (Vec<FollowedThread>, Vec<User>) {
    let mut users: BTreeMap<String, User> = BTreeMap::new();
    let mut out = Vec::new();
    for chat in v.get("Chats").and_then(Value::as_array).into_iter().flatten() {
        let Some((parent_chat, parent_ts)) = s(chat, &["ChatId"]).and_then(thread_parent) else {
            continue;
        };
        let Some(sm) = chat.get("ThreadParentMessage") else {
            continue;
        };
        let from = sm.pointer("/ServerMessageInfo/From").and_then(user_from);
        for u in from.into_iter().chain(mentioned_users(sm)) {
            users.entry(u.id.0.clone()).or_insert(u);
        }
        let Parsed::Message(mut parent) = parse_server_message(sm, Some(&parent_chat), me) else {
            continue;
        };
        // The parent lives in the chat itself, whatever `Plain.ChatId` says.
        parent.chat_id = ChatId(parent_chat);
        parent.ts = parent_ts;
        let last_seq = n(chat, &["LastSeqNo"]).unwrap_or(0);
        out.push(FollowedThread {
            unread: last_seq.saturating_sub(n(chat, &["LastSeenByMeSeqNo"]).unwrap_or(0)) as u32,
            last_ts: n(chat, &["LastTsMcs"]).unwrap_or(parent_ts),
            last_seq,
            parent: *parent,
        });
    }
    out.sort_by_key(|t| std::cmp::Reverse(t.last_ts));
    (out, users.into_values().collect())
}

/// UI event built from a Xiva push.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    /// New or changed message; `author` in case the UI does not know the sender, `mentioned`
    /// for the names of the people it mentions.
    Upsert {
        message: Box<Message>,
        author: Option<User>,
        #[serde(default)]
        mentioned: Vec<User>,
    },
    /// A message was deleted; `thread_parent` is set for a thread reply.
    Deleted {
        chat_id: ChatId,
        ts: Timestamp,
        #[serde(default)]
        thread_parent: Option<Timestamp>,
    },
    Typing {
        chat_id: ChatId,
        user: UserId,
    },
    /// Resulting reactions of a message after someone changed them; `users` are the people
    /// who reacted, for their names.
    Reactions {
        chat_id: ChatId,
        ts: Timestamp,
        reactions: Vec<Reaction>,
        #[serde(default)]
        users: Vec<User>,
    },
    /// The user read the chat (possibly on another device).
    Seen {
        chat_id: ChatId,
    },
    /// The user read the thread of message `parent` in `chat_id` (possibly on another device).
    ThreadSeen {
        chat_id: ChatId,
        parent: Timestamp,
    },
    /// A person came online (their `Heartbeat` arrived).
    Online {
        user: UserId,
        online: bool,
    },
    /// A person changed their availability or custom status.
    Status(PresenceInfo),
    /// A message was pinned in the chat (`ts`) or unpinned (`None`).
    Pinned {
        chat_id: ChatId,
        ts: Option<Timestamp>,
    },
    /// A chat the list did not have yet, for example a direct chat someone has just started:
    /// it goes into `section`; `users` are people the UI may not know yet.
    ChatAdded {
        chat: Chat,
        section: String,
        users: Vec<User>,
    },
    /// The connection to Telemost went up or down.
    Connection {
        online: bool,
    },
    /// Directory profiles (photo, position, real name) of authors that came with a history page.
    Users {
        users: Vec<User>,
    },
    /// The followed threads fetched again after a reply in a thread the list did not have.
    Threads {
        threads: Vec<FollowedThread>,
        users: Vec<User>,
    },
}

/// Parses the body of a `delivery` push. `None` means the event is not of interest yet.
pub fn from_push(body: &Value, me: Option<&UserId>) -> Option<LiveEvent> {
    let sm = body.get("ServerMessage").unwrap_or(body);
    let cm = sm.get("ClientMessage")?;
    if let Some(typing) = cm.get("Typing").or_else(|| cm.get("TypingEnhanced")) {
        let chat_id = ChatId(s(typing, &["ChatId"])?.to_owned());
        let user = UserId(s(sm, &["ServerMessageInfo", "From", "Guid"])?.to_owned());
        if me == Some(&user) {
            return None;
        }
        return Some(LiveEvent::Typing { chat_id, user });
    }
    let from = s(sm, &["ServerMessageInfo", "From", "Guid"]).map(|g| UserId(g.to_owned()));
    if cm.get("Heartbeat").is_some() {
        return Some(LiveEvent::Online {
            user: from?,
            online: true,
        });
    }
    if let Some(us) = cm.get("UserStatus") {
        let (availability, status) = user_status(us);
        return Some(LiveEvent::Status(PresenceInfo {
            user: from?,
            online: None,
            availability,
            status,
        }));
    }
    if let Some(r) = cm.get("Reaction") {
        let chat = s(r, &["ChatId"])?;
        return Some(LiveEvent::Reactions {
            chat_id: ChatId(thread_parent(chat).map_or_else(|| chat.to_owned(), |(p, _)| p)),
            ts: n(r, &["Timestamp"])?,
            reactions: reactions(sm, me),
            users: reaction_authors(sm),
        });
    }
    if let Some(seen) = cm.get("SeenMarker") {
        let chat = s(seen, &["ChatId"])?;
        if me.is_none_or(|me| from.as_ref() != Some(me)) {
            return None;
        }
        return Some(match thread_parent(chat) {
            Some((chat_id, parent)) => LiveEvent::ThreadSeen {
                chat_id: ChatId(chat_id),
                parent,
            },
            None => LiveEvent::Seen {
                chat_id: ChatId(chat.to_owned()),
            },
        });
    }
    if let Some(diff) = cm.pointer("/SystemMessage/ChatInfoDiff")
        && let Some(list) = diff.get("PinnedMessageTimestamps")
    {
        let chat = s(&cm["SystemMessage"], &["ChatId"])?;
        let ts = list.as_array().and_then(|l| l.iter().filter_map(Value::as_u64).max());
        return Some(LiveEvent::Pinned {
            chat_id: ChatId(chat.to_owned()),
            ts,
        });
    }
    match parse_server_message(sm, None, me) {
        Parsed::Message(message) => Some(LiveEvent::Upsert {
            author: sm.pointer("/ServerMessageInfo/From").and_then(user_from),
            mentioned: mentioned_users(sm),
            message,
        }),
        Parsed::Deleted {
            chat_id,
            ts,
            thread_parent,
        } => Some(LiveEvent::Deleted {
            chat_id,
            ts,
            thread_parent,
        }),
        Parsed::Skip => None,
    }
}

/// The sidebar section of a chat: `dms`, `meetings` or `channels`.
pub fn section_of(chat: &Chat) -> &'static str {
    match chat.kind {
        ChatKind::Direct { .. } | ChatKind::Notes => "dms",
        _ if is_meeting_chat(&chat.id.0) => "meetings",
        _ => "channels",
    }
}

/// A chat from a `get_chats_info` item: `{chat_id, name, description, is_public, members_count,
/// organization_ids}`. Counters and times are left for the messages to fill in.
pub fn chat_from_info(info: &Value, me: Option<&UserId>) -> Option<Chat> {
    let id = s(info, &["chat_id"])?;
    let kind = chat_kind(id, me, info.get("is_public").and_then(Value::as_bool).unwrap_or(false));
    // Like the history, direct chats belong to no organization: they show in all of them.
    let organizations = match kind {
        ChatKind::Direct { .. } | ChatKind::Notes => vec![],
        _ => info
            .get("organization_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .collect(),
    };
    Some(Chat {
        id: ChatId(id.to_owned()),
        kind,
        name: s(info, &["name"]).unwrap_or_default().to_owned(),
        description: s(info, &["description"]).filter(|d| !d.is_empty()).map(str::to_owned),
        member_count: n(info, &["members_count"]).unwrap_or(0) as u32,
        unread: 0,
        mentions: 0,
        muted: false,
        mute_mentions: false,
        last_ts: 0,
        pinned: None,
        organizations,
    })
}

/// Builds the UI snapshot: "Channels", "Direct messages" and "Meeting chats" sections.
pub fn snapshot(me: User, organization: String, mut data: HistoryData) -> Snapshot {
    data.chats.sort_by(|a, b| b.last_ts.cmp(&a.last_ts));
    let (mut channels, mut dms, mut meetings) = (Vec::new(), Vec::new(), Vec::new());
    // The chat with oneself, like "you" in Slack, always comes first among direct messages.
    if let Some(notes) = data.chats.iter().find(|c| c.kind == ChatKind::Notes) {
        dms.push(notes.id.clone());
    }
    for ch in data.chats.iter().filter(|ch| ch.kind != ChatKind::Notes) {
        match section_of(ch) {
            "dms" => dms.push(ch.id.clone()),
            "meetings" => meetings.push(ch.id.clone()),
            _ => channels.push(ch.id.clone()),
        }
    }
    let mut users = data.users;
    if !users.iter().any(|u| u.id == me.id) {
        users.push(me.clone());
    }
    // Direct chat peers that the history says nothing about.
    for ch in &data.chats {
        if let ChatKind::Direct { peer } = &ch.kind
            && !users.iter().any(|u| &u.id == peer)
        {
            users.push(User {
                id: peer.clone(),
                display_name: if ch.name.is_empty() {
                    UNKNOWN_USER.into()
                } else {
                    ch.name.clone()
                },
                title: None,
                presence: Presence::Offline,
                status: None,
                availability: Default::default(),
                is_robot: false,
                avatar: None,
            });
        }
    }
    let section = |id: &str, title: &str, chats| Section {
        id: id.into(),
        title: title.into(),
        chats,
    };
    Snapshot {
        me: me.id,
        organization,
        organization_avatar: None,
        organizations: vec![],
        users,
        chats: data.chats,
        sections: vec![
            section("channels", "Channels", channels),
            section("dms", "Direct messages", dms),
            section("meetings", "Meeting chats", meetings),
        ],
        messages: data.messages,
        folders: vec![],
    }
}

/// `UserStatus` → availability and custom status (emoji and text).
pub fn user_status(v: &Value) -> (Availability, Option<CustomStatus>) {
    let availability = Availability::from_code(n(v, &["Availability"]).unwrap_or(0));
    let status = v.get("CustomStatus").and_then(|c| {
        let emoji = s(c, &["Emoji"]).unwrap_or_default().to_owned();
        let text = s(c, &["Text"]).unwrap_or_default().to_owned();
        (!emoji.is_empty() || !text.is_empty()).then_some(CustomStatus {
            emoji,
            text,
            in_meeting: is_meeting_status(v),
        })
    });
    (availability, status)
}

/// Whether a `UserStatus` is the "In meeting" status that Telemost itself sets for the time of a
/// call (📞, `IconName: telemost`), and not one the user chose.
pub fn is_meeting_status(user_status: &Value) -> bool {
    let Some(c) = user_status.get("CustomStatus") else {
        return false;
    };
    s(c, &["IconName"]) == Some("telemost")
        || (s(c, &["Emoji"]) == Some("📞")
            && s(c, &["Localizations", "en"]).is_some_and(|t| t.eq_ignore_ascii_case("In meeting")))
}

/// Own raw `UserStatus` from the `subscribe` response with `MessageBodyTypes: [26]`.
pub fn own_user_status<'a>(v: &'a Value, me: &UserId) -> Option<&'a Value> {
    v.get("LastMessages")?
        .as_array()?
        .iter()
        .find(|m| s(m, &["ServerMessageInfo", "From", "Guid"]) == Some(me.0.as_str()))?
        .pointer("/ClientMessage/UserStatus")
}

/// Statuses from the `subscribe` response with `MessageBodyTypes: [26]`.
pub fn statuses_from_subscribe(v: &Value) -> Vec<PresenceInfo> {
    v.get("LastMessages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let user = UserId(s(m, &["ServerMessageInfo", "From", "Guid"])?.to_owned());
            let (availability, status) = user_status(m.pointer("/ClientMessage/UserStatus").unwrap_or(&Value::Null));
            Some(PresenceInfo {
                user,
                online: None,
                availability,
                status,
            })
        })
        .collect()
}

/// How many microseconds after the last `Heartbeat` a person counts as online. Other clients
/// (web, mobile, background tabs) send it less often than once a minute, so a short window makes
/// the dot blink between pushes; 10 minutes covers their interval.
pub const ONLINE_WINDOW_MCS: u64 = 10 * 60 * 1_000_000;

/// "Online" from the `subscribe` response with `MessageBodyType: 2` (last `Heartbeat` and `CurrentTime`).
pub fn online_from_heartbeat(v: &Value) -> Option<bool> {
    let last = n(v, &["LastMessage", "ServerMessageInfo", "Timestamp"])?;
    let now = n(v, &["CurrentTime"])?;
    Some(now.saturating_sub(last) < ONLINE_WINDOW_MCS)
}

/// `get_users_data` answers about at most this many people: the rest of a longer list is
/// silently left out (checked on 2026-11-11: 40 asked, the first 32 returned).
pub const PROFILES_PER_REQUEST: usize = 32;

/// Profiles from the HTTP RPC `get_users_data` response (`{guids: [...]}`).
pub fn profiles(v: &Value) -> Vec<UserProfile> {
    let text = |u: &Value, path: &[&str]| s(u, path).filter(|x| !x.is_empty()).map(str::to_owned);
    v.pointer("/data/users")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|u| {
            Some(UserProfile {
                id: UserId(s(u, &["guid"])?.to_owned()),
                display_name: text(u, &["display_name"]).unwrap_or_default(),
                email: text(u, &["email"]),
                position: text(u, &["employee_info", "position"]),
                department: text(u, &["employee_info", "department", "name"]),
                organization: u
                    .pointer("/employees_info/0/organization_name")
                    .and_then(Value::as_str)
                    .filter(|x| !x.is_empty())
                    .map(str::to_owned),
                nickname: text(u, &["nickname"]),
                avatar: text(u, &["avatar_id"]),
            })
        })
        .collect()
}

/// Own GUID from the chat list of a `history` response, for when `whoami` has failed: the chat
/// with oneself (`<me>_<me>`), otherwise the only GUID that every direct chat shares.
pub fn own_guid(history: &Value) -> Option<UserId> {
    let pairs: Vec<(&str, &str)> = history
        .get("Chats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| s(c, &["ChatId"]))
        .filter(|id| !id.contains('/'))
        .filter_map(|id| id.split_once('_'))
        .filter(|(a, b)| !a.is_empty() && !b.is_empty())
        .collect();
    if let Some((a, _)) = pairs.iter().find(|(a, b)| a == b) {
        return Some(UserId((*a).to_owned()));
    }
    let (a, b) = *pairs.first()?;
    let shared: Vec<&str> = [a, b]
        .into_iter()
        .filter(|g| pairs.iter().all(|(x, y)| x == g || y == g))
        .collect();
    match shared[..] {
        [g] => Some(UserId(g.to_owned())),
        _ => None,
    }
}

/// A `whoami`-shaped answer built from one's own directory profile, for when `whoami` has failed.
pub fn whoami_from_profile(me: &UserId, profile: Option<&UserProfile>) -> Value {
    let mut info = json!({ "Guid": me.0 });
    if let Some(p) = profile {
        info["DisplayName"] = Value::from(p.display_name.as_str());
        if let Some(a) = &p.avatar {
            info["AvatarId"] = Value::from(a.as_str());
        }
        if let Some(o) = &p.organization {
            info["OrganizationInfos"] = json!([{ "OrganizationName": o }]);
        }
    }
    json!({ "UserInfo": info })
}

/// Members from the `get_chat_members` response: the same records as `get_users_data`, plus `role`.
pub fn members(v: &Value) -> Vec<ChatMember> {
    let roles: BTreeMap<String, String> = v
        .pointer("/data/users")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|u| {
            Some((
                s(u, &["guid"])?.to_owned(),
                s(u, &["role"]).unwrap_or("member").to_owned(),
            ))
        })
        .collect();
    profiles(v)
        .into_iter()
        .map(|p| {
            let role = roles.get(&p.id.0).cloned().unwrap_or_else(|| "member".into());
            ChatMember { profile: p, role }
        })
        .collect()
}

/// ID of the direct chat with oneself.
pub fn notes_chat_id(me: &UserId) -> ChatId {
    ChatId(format!("{0}_{0}", me.0))
}

/// "Notes" chat for the snapshot when the history lacks it (just created, no messages yet).
pub fn notes_chat(me: &UserId) -> Chat {
    Chat {
        id: notes_chat_id(me),
        kind: ChatKind::Notes,
        name: String::new(),
        description: None,
        member_count: 1,
        unread: 0,
        mentions: 0,
        muted: false,
        mute_mentions: false,
        last_ts: 0,
        pinned: None,
        organizations: vec![],
    }
}

/// Whether a folder of the `folders` bucket is one of chat types ("Личные", "Новые", support
/// chats): a filter rather than a list of chats.
fn is_type_folder(f: &Value) -> bool {
    f.get("included_type_ids")
        .and_then(Value::as_array)
        .is_some_and(|l| !l.is_empty())
}

/// The folders listing their chats by hand from the `folders` bucket value:
/// `{"<org id>": {"custom_folders": [{id, name, included_chat_ids, …}], "main_folder": …}}`.
pub fn chat_folders(value: &Value) -> Vec<ChatFolder> {
    let Some(orgs) = value.as_object() else {
        return vec![];
    };
    orgs.iter()
        .flat_map(|(org, v)| {
            v.get("custom_folders")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|f| !is_type_folder(f))
                .filter_map(move |f| {
                    Some(ChatFolder {
                        id: s(f, &["id"])?.to_owned(),
                        org: org.clone(),
                        name: s(f, &["name"]).unwrap_or_default().to_owned(),
                        chats: f
                            .get("included_chat_ids")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(|id| ChatId(id.to_owned()))
                            .collect(),
                    })
                })
        })
        .collect()
}

/// Writes the organization's folders into the `folders` bucket value: they replace its folders
/// of chats in this order, keeping the fields Mostik does not know (icon, pinned chats); the
/// folders by chat type stay where they are, as do other organizations.
pub fn put_chat_folders(value: &mut Value, org: &str, folders: &[ChatFolder]) {
    if !value.is_object() {
        *value = json!({});
    }
    let entry = &mut value[org];
    if !entry.is_object() {
        *entry = json!({ "custom_folders": [], "main_folder": { "excluded_folder_ids": [] } });
    }
    let old: Vec<Value> = entry
        .get("custom_folders")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let write = |f: &ChatFolder| {
        let mut v = old
            .iter()
            .find(|o| s(o, &["id"]) == Some(f.id.as_str()) && !is_type_folder(o))
            .cloned()
            .unwrap_or_else(|| {
                json!({ "excluded_chat_ids": [], "excluded_type_ids": [], "included_type_ids": [],
                        "pinned_chat_ids": [] })
            });
        let chats: Vec<&str> = f.chats.iter().map(|c| c.0.as_str()).collect();
        if let Some(pinned) = v.get_mut("pinned_chat_ids").and_then(Value::as_array_mut) {
            pinned.retain(|p| p.as_str().is_some_and(|p| chats.contains(&p)));
        }
        v["id"] = json!(f.id);
        v["name"] = json!(f.name);
        v["included_chat_ids"] = json!(chats);
        v
    };
    let mut mine = folders.iter().map(write);
    let mut list: Vec<Value> = old
        .iter()
        .filter_map(|o| {
            if is_type_folder(o) {
                Some(o.clone())
            } else {
                mine.next()
            }
        })
        .collect();
    list.extend(mine);
    entry["custom_folders"] = json!(list);
}

/// Applies synced settings (`get_buckets`): chat folders, pinned chats go to the "Starred"
/// section, muted chats set `Chat::muted`.
pub fn apply_buckets(snapshot: &mut Snapshot, buckets: &Value) {
    let list = buckets
        .pointer("/data/buckets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let bucket = |name: &str| {
        list.iter()
            .find(|b| b["bucket_name"] == name)
            .map(|b| b["bucket_value"].clone())
    };
    snapshot.folders = bucket("folders").map(|v| chat_folders(&v)).unwrap_or_default();
    if let Some(mutings) = bucket("chat_mutings").and_then(|v| v.as_object().cloned()) {
        for ch in &mut snapshot.chats {
            if let Some(m) = mutings.get(&ch.id.0) {
                ch.muted = m.get("mute").and_then(Value::as_bool).unwrap_or(ch.muted);
                ch.mute_mentions = m.get("mute_mentions").and_then(Value::as_bool).unwrap_or(false);
            }
        }
    }
    let pinned: Vec<ChatId> = bucket("pinned_chats")
        .and_then(|v| v.get("pinned_chats").cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(|id| ChatId(id.to_owned()))
        .filter(|id| snapshot.chat(id).is_some())
        .collect();
    if pinned.is_empty() {
        return;
    }
    for sec in &mut snapshot.sections {
        sec.chats.retain(|id| !pinned.contains(id));
    }
    snapshot.sections.insert(
        0,
        Section {
            id: "starred".into(),
            title: "Starred".into(),
            chats: pinned,
        },
    );
}

#[cfg(test)]
mod tests {

    #[test]
    fn direct_chat_peer_is_named_by_partner_info() {
        let me = UserId("me".into());
        let list = json!({"Chats": [{"ChatId": "os_me", "LastSeqNo": 1,
            "PartnerInfo": {"DisplayName": "Olga Sokolova", "Guid": "os", "PublicName": "Olga Sokolova"},
            "Messages": [{"ServerMessage": {"ClientMessage": {"Plain": {"ChatId": "os_me", "Text": {"MessageText": "hi"}}},
                "ServerMessageInfo": {"From": {"DisplayName": "Me", "Guid": "me"}, "SeqNo": 1, "Timestamp": 5}}}]}]});
        let data = from_history(&list, Some(&me));
        let peer = data
            .users
            .iter()
            .find(|u| u.id.0 == "os")
            .map(|u| u.display_name.as_str());
        assert_eq!(peer, Some("Olga Sokolova"));
    }

    #[test]
    fn meeting_status_is_telemost_own() {
        let call = json!({"CustomStatus": {"Emoji": "📞", "Text": "На встрече",
            "Localizations": {"en": "In meeting", "ru": "На встрече"}, "IconName": "telemost"}});
        assert!(is_meeting_status(&call));
        let without_icon = json!({"CustomStatus": {"Emoji": "📞", "Text": "На встрече",
            "Localizations": {"en": "In meeting"}}});
        assert!(is_meeting_status(&without_icon));
        // The user's own phone status stays.
        assert!(!is_meeting_status(
            &json!({"CustomStatus": {"Emoji": "📞", "Text": "Созвоны до обеда"}})
        ));
        assert!(!is_meeting_status(&json!({"Availability": 3})));
    }

    #[test]
    fn followed_threads_with_unread() {
        let v = json!({"Chats": [
            {"ChatId": "100/0/abc_42", "LastSeqNo": 5, "LastSeenByMeSeqNo": 3, "LastTsMcs": 90,
             "ThreadParentMessage": {"ClientMessage": {"Plain": {"ChatId": "0/0/abc", "Text": {"MessageText": "plan"}}},
                "ServerMessageInfo": {"Timestamp": 42, "From": {"Guid": "u1", "DisplayName": "Anna"},
                    "ThreadState": {"LastSeqNo": 5, "LastTsMcs": 90}}}},
            {"ChatId": "100/0/def_7", "LastSeqNo": 2, "LastSeenByMeSeqNo": 2, "LastTsMcs": 95,
             "ThreadParentMessage": {"ClientMessage": {"Plain": {"ChatId": "0/0/def", "Text": {"MessageText": "q"}}},
                "ServerMessageInfo": {"Timestamp": 7, "From": {"Guid": "u2"}}}},
            {"ChatId": "0/0/not-a-thread", "LastSeqNo": 1}
        ]});
        let (threads, users) = followed_threads(&v, None);
        assert_eq!(threads.len(), 2);
        assert_eq!(
            (
                threads[0].parent.chat_id.0.as_str(),
                threads[0].parent.ts,
                threads[0].unread
            ),
            ("0/0/def", 7, 0)
        );
        let t = &threads[1];
        assert_eq!(
            (t.parent.chat_id.0.as_str(), t.parent.ts, t.parent.text.as_str()),
            ("0/0/abc", 42, "plan")
        );
        assert_eq!((t.unread, t.last_ts, t.last_seq), (2, 90, 5));
        assert_eq!(t.parent.thread.as_ref().map(|t| t.reply_count), Some(5));
        assert_eq!(users.len(), 2);
        let me = UserId("me".into());
        let seen = json!({"ClientMessage": {"SeenMarker": {"ChatId": "100/0/abc_42", "SeqNo": 5}}, "ServerMessageInfo": {"From": {"Guid": "me"}}});
        assert_eq!(
            from_push(&seen, Some(&me)),
            Some(LiveEvent::ThreadSeen {
                chat_id: ChatId("0/0/abc".into()),
                parent: 42
            })
        );
    }

    #[test]
    fn chat_from_chats_info() {
        let me = UserId("aa".into());
        let group = json!({"chat_id": "0/0/x", "name": "dev", "description": "", "is_public": true,
            "members_count": 19, "organization_ids": [7]});
        let ch = chat_from_info(&group, Some(&me)).unwrap();
        assert_eq!(ch.kind, ChatKind::Group { public: true });
        assert_eq!(
            (ch.name.as_str(), ch.description.as_deref(), ch.member_count),
            ("dev", None, 19)
        );
        assert_eq!(section_of(&ch), "channels");
        assert_eq!(ch.organizations, vec![7]);
        let dm = json!({"chat_id": "bb_aa", "members_count": 2, "organization_ids": [7], "public": false});
        let ch = chat_from_info(&dm, Some(&me)).unwrap();
        assert_eq!(
            ch.kind,
            ChatKind::Direct {
                peer: UserId("bb".into())
            }
        );
        assert!(ch.organizations.is_empty());
        assert_eq!(section_of(&ch), "dms");
        let meeting = json!({"chat_id": "0/22/m", "name": "Sync"});
        assert_eq!(section_of(&chat_from_info(&meeting, Some(&me)).unwrap()), "meetings");
    }

    #[test]
    fn meeting_messages_are_call_notes() {
        let push = |kind: &str, id: &str| {
            json!({"ServerMessage": {"ClientMessage": {"NotificationBehaviour": 2, "SystemMessage": {
                    "ChatId": "0/0/abc", kind: {"MeetingId": id}, "PayloadId": "p"}},
                "ServerMessageInfo": {"From": {"DisplayName": "Sergey", "Guid": "u1"}, "SeqNo": 51, "Timestamp": 7}}})
        };
        match from_push(
            &push("MeetingStartedMessage", "https://telemost.360.yandex.ru/j/123"),
            None,
        ) {
            Some(LiveEvent::Upsert { message, author, .. }) => {
                assert_eq!(message.chat_id, ChatId("0/0/abc".into()));
                assert_eq!((message.ts, message.seq_no, message.author.0.as_str()), (7, 51, "u1"));
                assert_eq!(
                    message.call,
                    Some(CallNote::Started {
                        url: "https://telemost.360.yandex.ru/j/123".into()
                    })
                );
                assert_eq!(author.map(|a| a.display_name), Some("Sergey".into()));
            }
            other => panic!("{other:?}"),
        }
        match from_push(&push("MeetingEndedMessage", "123"), None) {
            Some(LiveEvent::Upsert { message, .. }) => assert_eq!(
                message.call,
                Some(CallNote::Ended {
                    url: "https://telemost.360.yandex.ru/j/123".into(),
                    reason: None,
                })
            ),
            other => panic!("{other:?}"),
        }
        let joined = json!({"ClientMessage": {"SystemMessage": {"ChatId": "0/0/abc", "UserAction": 0}},
            "ServerMessageInfo": {"From": {"Guid": "u1"}, "Timestamp": 8}});
        assert_eq!(from_push(&joined, None), None);
    }

    #[test]
    fn direct_calls_are_call_notes() {
        let chat = "aa_bb";
        let url = "https://telemost.360.yandex.ru/j/42";
        // The ringing repeats every 5 s with `IsSilent`; every repeat is the same note.
        let ring = |silent: bool, sent: u64| {
            json!({"ClientMessage": {"IsSilent": silent, "MeetingCallingMessage": {
                    "IncomingCall": {"CallSettings": {"Mic": true}, "MeetingInfo": {"ChatId": chat,
                        "CreateTimestamp": 100, "MeetingId": url, "Version": 1}},
                    "RingingId": "r1"}, "NotificationBehaviour": 2},
                "ServerMessageInfo": {"From": {"DisplayName": "Boris", "Guid": "bb"}, "Timestamp": sent}})
        };
        for (silent, sent) in [(false, 150), (true, 5_000_150)] {
            assert_eq!(
                ringing(&ring(silent, sent)),
                Some(Ringing::Incoming { id: "r1".into() })
            );
            match from_push(&ring(silent, sent), Some(&UserId("aa".into()))) {
                Some(LiveEvent::Upsert { message, author, .. }) => {
                    assert_eq!(message.chat_id, ChatId(chat.into()));
                    assert_eq!((message.ts, message.seq_no, message.author.0.as_str()), (100, 0, "bb"));
                    assert_eq!(message.call, Some(CallNote::Ringing { url: url.into() }));
                    assert!(message.is_ringing());
                    assert_eq!(author.map(|a| a.display_name), Some("Boris".into()));
                }
                other => panic!("{other:?}"),
            }
        }
        let ended = json!({"ClientMessage": {"NotificationBehaviour": 2, "SystemMessage": {"ChatId": chat,
                "IncrementUnreadCounter": true, "PayloadId": "p",
                "PersonalMeetingEndedMessage": {"CallType": 1, "MeetingId": url, "Reason": 3}}},
            "ServerMessageInfo": {"From": {"Guid": "bb"}, "SeqNo": 50, "Timestamp": 900}});
        match from_push(&ended, None) {
            Some(LiveEvent::Upsert { message, .. }) => {
                assert_eq!((message.ts, message.seq_no), (900, 50));
                assert_eq!(
                    message.call,
                    Some(CallNote::Ended {
                        url: url.into(),
                        reason: Some(CallEndReason::Timeout),
                    })
                );
                assert!(!message.is_ringing());
            }
            other => panic!("{other:?}"),
        }
        // A declined call: Telemost leaves out the zero `Reason`.
        let declined = json!({"ClientMessage": {"SystemMessage": {"ChatId": chat, "PayloadId": "p",
                "PersonalMeetingEndedMessage": {"CallType": 1, "MeetingId": url}}},
            "ServerMessageInfo": {"From": {"Guid": "bb"}, "SeqNo": 51, "Timestamp": 990}});
        match from_push(&declined, None) {
            Some(LiveEvent::Upsert { message, .. }) => assert_eq!(
                message.call,
                Some(CallNote::Ended {
                    url: url.into(),
                    reason: Some(CallEndReason::Declined),
                })
            ),
            other => panic!("{other:?}"),
        }
        let rang_off = json!({"ClientMessage": {"MeetingCallingMessage": {"RingingEnded": {}, "RingingId": "r1"}},
            "ServerMessageInfo": {"From": {"Guid": "11111111-1111-1111-1111-111111111111"}, "Timestamp": 950}});
        assert_eq!(from_push(&rang_off, None), None);
        assert_eq!(ringing(&rang_off), Some(Ringing::Ended { id: "r1".into() }));
        assert_eq!(ringing(&ended), None);
    }

    #[test]
    fn meeting_status_is_marked() {
        let (_, st) =
            user_status(&json!({"CustomStatus": {"Emoji": "📞", "Text": "На встрече", "IconName": "telemost"}}));
        assert!(st.unwrap().in_meeting);
        let (_, st) = user_status(&json!({"CustomStatus": {"Emoji": "🏖", "Text": "Vacation"}}));
        assert!(!st.unwrap().in_meeting);
    }

    #[test]
    fn own_status_from_subscribe() {
        let v = json!({"LastMessages": [
            {"ClientMessage": {"UserStatus": {"Availability": 1}}, "ServerMessageInfo": {"From": {"Guid": "other"}}},
            {"ClientMessage": {"UserStatus": {"Availability": 3}}, "ServerMessageInfo": {"From": {"Guid": "me"}}}]});
        assert_eq!(own_user_status(&v, &UserId("me".into())).unwrap()["Availability"], 3);
        assert!(own_user_status(&v, &UserId("nobody".into())).is_none());
    }

    use super::*;
    use serde_json::json;

    /// A message of `chat` forwarding `refs` (`(chat, ts, text)`) with the `styles` the web client sends.
    fn with_forwards(chat: &str, refs: &[(&str, u64, &str)], styles: Value) -> Message {
        let forwarded: Vec<Value> = refs
            .iter()
            .map(|(c, ts, text)| {
                json!({"Payload": {"ChatId": c, "Text": {"MessageText": text}},
                    "ServerMessageInfo": {"From": {"Guid": "a", "DisplayName": "Anna"}, "Timestamp": ts}})
            })
            .collect();
        let sm = json!({"ClientMessage": {"Plain": {"ChatId": chat, "Text": {"MessageText": "answer"},
                "ForwardedMessageStyles": styles}},
            "ForwardedMessages": forwarded,
            "ServerMessageInfo": {"From": {"Guid": "b"}, "Timestamp": 100, "SeqNo": 3}});
        match parse_server_message(&sm, None, None) {
            Parsed::Message(m) => *m,
            _ => panic!("not a message"),
        }
    }

    #[test]
    fn replies_and_quotes_from_the_web_client() {
        let reply = with_forwards("0/0/g", &[("0/0/g", 42, "original")], json!([{}]));
        let to = reply.reply_to().expect("a reply");
        assert_eq!(
            (to.ts, to.text.as_str(), to.author_name.as_str()),
            (42, "original", "Anna")
        );
        assert_eq!(to.quote, None);
        assert_eq!(reply.text, "answer");

        let quote = with_forwards("0/0/g", &[("0/0/g", 42, "original")], json!([{"Quote": "orig"}]));
        assert_eq!(quote.reply_to().and_then(|f| f.quote.as_deref()), Some("orig"));

        // In a thread both the reply and the original are in the thread chat.
        let in_thread = with_forwards("100/0/g_7", &[("100/0/g_7", 42, "original")], json!([]));
        assert_eq!(in_thread.thread_parent, Some(7));
        assert!(in_thread.reply_to().is_some());

        // A quote from another chat is a reply too; a plain forward from another chat is not.
        let quoted = with_forwards("0/0/g", &[("0/0/other", 42, "x")], json!([{"Quote": "x"}]));
        assert!(quoted.reply_to().is_some());
        let forward = with_forwards("0/0/g", &[("0/0/other", 42, "x")], json!([{}]));
        assert!(forward.reply_to().is_none());
        assert_eq!(forward.forwarded.len(), 1);
        let two = with_forwards(
            "0/0/g",
            &[("0/0/g", 1, "a"), ("0/0/g", 2, "b")],
            json!([{}, {"Quote": "b"}]),
        );
        assert!(two.reply_to().is_none());
        assert_eq!(two.forwarded[1].quote.as_deref(), Some("b"));
    }

    #[test]
    fn a_reply_to_a_deleted_message_keeps_no_text() {
        let sm = json!({"ClientMessage": {"Plain": {"ChatId": "0/0/g", "Text": {"MessageText": "answer"}}},
            "ForwardedMessages": [{"Payload": {"ChatId": "0/0/g", "Timestamp": 42},
                "ServerMessageInfo": {"From": {"Guid": "a"}, "Timestamp": 42}}],
            "ServerMessageInfo": {"From": {"Guid": "b"}, "Timestamp": 100}});
        let Parsed::Message(m) = parse_server_message(&sm, None, None) else {
            panic!("not a message")
        };
        let to = m.reply_to().expect("a reply");
        assert!(to.text.is_empty() && to.attachments.is_empty());
    }

    #[test]
    fn own_guid_from_the_notes_chat_or_the_shared_member() {
        let chats = |ids: &[&str]| json!({ "Chats": ids.iter().map(|id| json!({ "ChatId": id })).collect::<Vec<_>>() });
        let me = Some(UserId("me".into()));
        assert_eq!(own_guid(&chats(&["0/0/g", "a_me", "me_me"])), me);
        assert_eq!(own_guid(&chats(&["a_me", "me_b", "0/0/g"])), me);
        // One direct chat says nothing about which side is ours.
        assert_eq!(own_guid(&chats(&["a_me", "0/0/g"])), None);
        assert_eq!(own_guid(&chats(&["0/0/g"])), None);
    }

    #[test]
    fn whoami_from_the_directory_profile() {
        let me = UserId("me".into());
        let profile = UserProfile {
            id: me.clone(),
            display_name: "Ivan".into(),
            email: None,
            position: None,
            department: None,
            organization: Some("Org".into()),
            nickname: None,
            avatar: Some("1/abc".into()),
        };
        let who = whoami_from_profile(&me, Some(&profile));
        assert_eq!(who.pointer("/UserInfo/Guid"), Some(&json!("me")));
        assert_eq!(who.pointer("/UserInfo/DisplayName"), Some(&json!("Ivan")));
        assert_eq!(who.pointer("/UserInfo/AvatarId"), Some(&json!("1/abc")));
        assert_eq!(
            who.pointer("/UserInfo/OrganizationInfos/0/OrganizationName"),
            Some(&json!("Org"))
        );
        assert_eq!(whoami_from_profile(&me, None), json!({ "UserInfo": { "Guid": "me" } }));
    }

    #[test]
    fn kinds_by_id() {
        let me = UserId("aa".into());
        assert_eq!(chat_kind("0/0/x-y", Some(&me), true), ChatKind::Group { public: true });
        assert_eq!(
            chat_kind("aa_bb", Some(&me), false),
            ChatKind::Direct {
                peer: UserId("bb".into())
            }
        );
        assert_eq!(
            chat_kind("bb_aa", Some(&me), false),
            ChatKind::Direct {
                peer: UserId("bb".into())
            }
        );
        assert_eq!(chat_kind("aa_aa", Some(&me), false), ChatKind::Notes);
        assert_eq!(chat_kind("1/0/x", Some(&me), true), ChatKind::Broadcast);
        assert!(is_meeting_chat("0/22/x") && !is_meeting_chat("0/0/x"));
        assert_eq!(thread_chat_id("0/0/abc-1", 42).as_deref(), Some("100/0/abc-1_42"));
        assert_eq!(thread_chat_id("aa_bb", 42), None);
        assert_eq!(thread_parent("100/0/abc-1_42"), Some(("0/0/abc-1".into(), 42)));
        let reply = json!({"ClientMessage": {"Plain": {"ChatId": "100/0/abc_42", "Text": {"MessageText": "r"}}},
                           "ServerMessageInfo": {"Timestamp": 50, "From": {"Guid": "u1"}}});
        match from_push(&reply, None) {
            Some(LiveEvent::Upsert { message, .. }) => {
                assert_eq!(message.chat_id, ChatId("0/0/abc".into()));
                assert_eq!(message.thread_parent, Some(42));
            }
            other => panic!("{other:?}"),
        }
        let parent = json!({"ClientMessage": {"Plain": {"ChatId": "0/0/abc", "Text": {"MessageText": "p"}}},
                            "ServerMessageInfo": {"Timestamp": 42, "ThreadState": {"LastSeqNo": 3, "LastTsMcs": 60}, "From": {"Guid": "u1"}}});
        match from_push(&parent, None) {
            Some(LiveEvent::Upsert { message, .. }) => {
                let t = message.thread.expect("thread summary");
                assert_eq!((t.reply_count, t.last_reply_ts), (3, 60));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parses_history_like_observed_schema() {
        let v = json!({"Chats": [{
            "ChatId": "0/0/abc",
            "ChatInfo": {"Name": "dev", "IsPublic": false, "MemberCount": 19, "Description": ""},
            "LastSeqNo": 10, "LastSeenByMeSeqNo": 7, "Muted": true, "LastTsMcs": 5,
            "Messages": [
                {"ServerMessage": {"ClientMessage": {"SystemMessage": {"ChatId": "0/0/abc"}},
                  "ServerMessageInfo": {"Timestamp": 1, "From": {"Guid": "u0"}}}},
                {"ServerMessage": {"ClientMessage": {"Plain": {"ChatId": "0/0/abc", "Timestamp": 3}},
                  "ServerMessageInfo": {"Timestamp": 3, "From": {"Guid": "u1"}}}},
                {"ServerMessage": {"ClientMessage": {"Plain": {"ChatId": "0/0/abc", "Text": {"MessageText": "hello"}}},
                  "ServerMessageInfo": {"Timestamp": 9, "SeqNo": 10, "Version": 1, "From": {"Guid": "u1", "DisplayName": "Anna"}},
                  "Reactions": [{"Type": 100102, "Count": 2}, {"Type": 100104, "Count": 0}],
                  "RecentUserReactions": [{"Type": 100102, "UserInfo": {"Guid": "me"}}],
                  "RecentUserReads": [{"UserInfo": {"Guid": "u2", "DisplayName": "Boris"}}]}}
            ]
        }]});
        let d = from_history(&v, Some(&UserId("me".into())));
        assert_eq!(d.chats.len(), 1);
        let ch = &d.chats[0];
        assert_eq!((ch.unread, ch.member_count, ch.muted, ch.last_ts), (3, 19, true, 9));
        assert_eq!(ch.description, None);
        assert_eq!(d.messages.len(), 1, "system and deleted messages stay out of the feed");
        assert_eq!(d.messages[0].text, "hello");
        assert_eq!(d.messages[0].seq_no, 10);
        assert_eq!(
            d.messages[0].reactions,
            vec![Reaction {
                kind: 100102,
                emoji: "👍".into(),
                count: 2,
                mine: true,
                users: vec![UserId("me".into())],
            }]
        );
        // Authors, readers and the one who reacted.
        assert_eq!(d.users.len(), 4);
        assert_eq!(d.last["0/0/abc"], (9, 10));
    }

    #[test]
    fn notes_chat_goes_first_in_dms() {
        let me_id = UserId("me".into());
        let me = User {
            id: me_id.clone(),
            display_name: "Me".into(),
            title: None,
            presence: Presence::Online,
            status: None,
            availability: Default::default(),
            is_robot: false,
            avatar: None,
        };
        let history = json!({"Chats": [
            {"ChatId": "me_u1", "LastTsMcs": 10},
            {"ChatId": "me_me", "LastTsMcs": 1}
        ]});
        let snap = snapshot(me, "org".into(), from_history(&history, Some(&me_id)));
        let dms = snap.sections.iter().find(|s| s.id == "dms").unwrap();
        assert_eq!(dms.chats[0], notes_chat_id(&me_id));
        assert_eq!(snap.chat(&notes_chat_id(&me_id)).unwrap().kind, ChatKind::Notes);
    }

    #[test]
    fn parses_profiles() {
        let v = json!({"status": "ok", "data": {"users": [{"guid": "g1", "display_name": "Anna", "email": "a@x.ru",
            "employee_info": {"position": "Designer", "department": {"id": "1", "name": "Product"}},
            "employees_info": [{"organization_name": "Org"}], "nickname": "", "avatar_id": "1/abc"}]}});
        let p = &profiles(&v)[0];
        assert_eq!(p.id, UserId("g1".into()));
        assert_eq!(p.position.as_deref(), Some("Designer"));
        assert_eq!(p.department.as_deref(), Some("Product"));
        assert_eq!(p.organization.as_deref(), Some("Org"));
        assert_eq!(p.nickname, None);
    }

    #[test]
    fn parses_members_with_roles() {
        let v = json!({"status": "ok", "data": {"users": [
            {"guid": "a", "display_name": "A", "role": "admin"},
            {"guid": "b", "display_name": "B"}
        ]}});
        let m = members(&v);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].role.as_str(), m[1].role.as_str()), ("admin", "member"));
    }

    #[test]
    fn gallery_attachments() {
        let v = json!({"ClientMessage": {"Plain": {"ChatId": "c", "Gallery": {"Text": "caption", "Items": [
            {"Image": {"FileInfo": {"Id2": "disk/x1", "Name": "a.png", "Size": 10}, "Width": 800, "Height": 600}},
            {"Image": {"FileInfo": {"Id2": "disk/x2", "Name": "b.png", "Size": 20}, "Width": 10, "Height": 10}}]}}},
            "ServerMessageInfo": {"Timestamp": 1, "From": {"Guid": "u"}}});
        match from_push(&v, None) {
            Some(LiveEvent::Upsert { message, .. }) => {
                assert_eq!(message.text, "caption");
                assert_eq!(message.attachments.len(), 2);
                assert_eq!(message.attachments[0].file_id, "disk/x1");
                assert_eq!(
                    (message.attachments[0].width, message.attachments[0].height),
                    (800, 600)
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn presence_from_subscribe() {
        let st = json!({"LastMessages": [{"ClientMessage": {"UserStatus": {}}, "ServerMessageInfo": {"From": {"Guid": "a"}}},
                                          {"ClientMessage": {"UserStatus": {"Availability": 1}}, "ServerMessageInfo": {"From": {"Guid": "b"}}}]});
        let v = statuses_from_subscribe(&st);
        assert_eq!(
            (v[0].availability, v[1].availability),
            (Availability::Default, Availability::Away)
        );
        let hb = |ago: u64| json!({"CurrentTime": 10_000_000_000u64, "LastMessage": {"ServerMessageInfo": {"Timestamp": 10_000_000_000u64 - ago}}});
        assert_eq!(online_from_heartbeat(&hb(60_000_000)), Some(true));
        assert_eq!(online_from_heartbeat(&hb(5 * 60_000_000)), Some(true));
        assert_eq!(online_from_heartbeat(&hb(15 * 60_000_000)), Some(false));
        assert_eq!(online_from_heartbeat(&json!({})), None);
    }

    #[test]
    fn folders_of_chats_are_read_and_written() {
        let mut value = json!({
            "0": {"custom_folders": [{"icon": {"value": 2}, "id": "default_private",
                "included_type_ids": ["private"], "name": "Личные"}]},
            "42": {"custom_folders": [
                {"icon": {"value": 1}, "id": "default_unread", "included_type_ids": ["unread"], "name": "Новые"},
                {"id": "a", "name": "⚠️|Test", "included_chat_ids": ["0/0/x", "0/0/y"],
                 "included_type_ids": [], "pinned_chat_ids": ["0/0/y"], "icon": {"value": 7}},
                {"id": "b", "name": "old", "included_chat_ids": []}],
                "main_folder": {"excluded_folder_ids": []}}});
        let folders = chat_folders(&value);
        assert_eq!(folders.len(), 2);
        assert_eq!((folders[0].org.as_str(), folders[0].name.as_str()), ("42", "⚠️|Test"));
        assert_eq!(folders[0].chats, vec![ChatId("0/0/x".into()), ChatId("0/0/y".into())]);

        // "b" deleted, "a" renamed with "0/0/y" taken out, "c" new.
        let new = vec![
            ChatFolder {
                id: "c".into(),
                org: "42".into(),
                name: "New".into(),
                chats: vec![ChatId("0/0/z".into())],
            },
            ChatFolder {
                id: "a".into(),
                org: "42".into(),
                name: "Work".into(),
                chats: vec![ChatId("0/0/x".into())],
            },
        ];
        put_chat_folders(&mut value, "42", &new);
        let list = value["42"]["custom_folders"].as_array().unwrap();
        let ids: Vec<&str> = list.iter().map(|f| f["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["default_unread", "c", "a"]);
        assert_eq!(list[2]["icon"]["value"], 7);
        assert_eq!(list[2]["pinned_chat_ids"], json!([]));
        assert_eq!(list[1]["included_chat_ids"], json!(["0/0/z"]));
        assert_eq!(value["42"]["main_folder"], json!({"excluded_folder_ids": []}));
        assert_eq!(value["0"]["custom_folders"][0]["id"], "default_private");
        assert_eq!(
            chat_folders(&value).iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["New", "Work"]
        );

        // An organization without folders yet.
        put_chat_folders(&mut value, "7", &new[..1]);
        assert_eq!(value["7"]["custom_folders"][0]["name"], "New");
    }

    #[test]
    fn buckets_pin_and_mute() {
        let me = User {
            id: UserId("me".into()),
            display_name: "Me".into(),
            title: None,
            presence: Presence::Online,
            status: None,
            availability: Default::default(),
            is_robot: false,
            avatar: None,
        };
        let history = json!({"Chats": [{"ChatId": "0/0/a", "ChatInfo": {"Name": "a"}}, {"ChatId": "0/0/b", "ChatInfo": {"Name": "b"}}]});
        let mut snap = snapshot(me, "org".into(), from_history(&history, None));
        let buckets = json!({"status": "ok", "data": {"buckets": [
            {"bucket_name": "pinned_chats", "bucket_value": {"pinned_chats": ["0/0/b", "0/0/missing"]}},
            {"bucket_name": "chat_mutings", "bucket_value": {"0/0/a": {"mute": true, "mute_mentions": false},
                                                              "0/0/b": {"mute": true, "mute_mentions": true}}}
        ]}});
        apply_buckets(&mut snap, &buckets);
        assert_eq!(snap.sections[0].id, "starred");
        assert_eq!(snap.sections[0].chats, vec![ChatId("0/0/b".into())]);
        assert!(!snap.sections[1].chats.contains(&ChatId("0/0/b".into())));
        assert!(snap.chat(&ChatId("0/0/a".into())).unwrap().muted);
        // Telemost's web client mutes with `mute` alone: mentions still notify.
        assert_eq!(
            snap.chat(&ChatId("0/0/a".into())).unwrap().notify_level(),
            mostik_core::NotifyLevel::Mentions
        );
        assert_eq!(
            snap.chat(&ChatId("0/0/b".into())).unwrap().notify_level(),
            mostik_core::NotifyLevel::Nothing
        );
    }

    #[test]
    fn push_events() {
        let me = UserId("me".into());
        let typing = json!({"ClientMessage": {"Typing": {"ChatId": "c"}}, "ServerMessageInfo": {"From": {"Guid": "u1"}, "Timestamp": 1}});
        assert_eq!(
            from_push(&typing, Some(&me)),
            Some(LiveEvent::Typing {
                chat_id: ChatId("c".into()),
                user: UserId("u1".into())
            })
        );
        let mine = json!({"ClientMessage": {"Typing": {"ChatId": "c"}}, "ServerMessageInfo": {"From": {"Guid": "me"}}});
        assert_eq!(from_push(&mine, Some(&me)), None);
        let new = json!({"ClientMessage": {"Plain": {"ChatId": "c", "Text": {"MessageText": "hi"}}},
                         "ServerMessageInfo": {"From": {"Guid": "u1", "DisplayName": "Anna"}, "Timestamp": 7, "SeqNo": 3}});
        match from_push(&new, Some(&me)) {
            Some(LiveEvent::Upsert { message, author, .. }) => {
                assert_eq!((message.ts, message.text.as_str(), message.seq_no), (7, "hi", 3));
                assert_eq!(author.unwrap().display_name, "Anna");
            }
            other => panic!("{other:?}"),
        }
        // Telemost names the people a message mentions as `@<guid>` in `MentionedUsers`.
        let mention = json!({"ClientMessage": {"Plain": {"ChatId": "c", "Text": {"MessageText": "@u2 look"}}},
                             "MentionedUsers": [{"Guid": "u2", "DisplayName": "Boris", "Version": 1}],
                             "ServerMessageInfo": {"From": {"Guid": "u1"}, "Timestamp": 8}});
        match from_push(&mention, Some(&me)) {
            Some(LiveEvent::Upsert { mentioned, .. }) => {
                assert_eq!(
                    mentioned
                        .iter()
                        .map(|u| (u.id.0.as_str(), u.display_name.as_str()))
                        .collect::<Vec<_>>(),
                    vec![("u2", "Boris")]
                );
            }
            other => panic!("{other:?}"),
        }
        let del = json!({"ClientMessage": {"Plain": {"ChatId": "c", "Timestamp": 7}}, "ServerMessageInfo": {"From": {"Guid": "u1"}, "Timestamp": 7}});
        assert_eq!(
            from_push(&del, Some(&me)),
            Some(LiveEvent::Deleted {
                chat_id: ChatId("c".into()),
                ts: 7,
                thread_parent: None
            })
        );
        // A deleted thread reply names its thread: the parent's reply count goes down.
        let reply = json!({"ClientMessage": {"Plain": {"ChatId": "100/0/abc_42", "Timestamp": 9}},
                           "ServerMessageInfo": {"From": {"Guid": "u1"}, "Timestamp": 9}});
        assert_eq!(
            from_push(&reply, Some(&me)),
            Some(LiveEvent::Deleted {
                chat_id: ChatId("0/0/abc".into()),
                ts: 9,
                thread_parent: Some(42)
            })
        );
        let del2 = json!({"ClientMessage": {"Plain": {"ChatId": "c", "PayloadId": "p"}}, "ServerMessageInfo": {"Deleted": true, "Timestamp": 8, "From": {"Guid": "u1"}}});
        assert_eq!(
            from_push(&del2, Some(&me)),
            Some(LiveEvent::Deleted {
                chat_id: ChatId("c".into()),
                ts: 8,
                thread_parent: None
            })
        );
        let react = json!({"ClientMessage": {"Reaction": {"ChatId": "c", "Timestamp": 7, "Type": 100102, "Action": 2}},
                           "Reactions": [{"Type": 100102, "Count": 1}], "RecentUserReactions": [{"Type": 100102, "UserInfo": {"Guid": "me"}}]});
        assert_eq!(
            from_push(&react, Some(&me)),
            Some(LiveEvent::Reactions {
                chat_id: ChatId("c".into()),
                ts: 7,
                reactions: vec![Reaction {
                    kind: 100102,
                    emoji: "👍".into(),
                    count: 1,
                    mine: true,
                    users: vec![me.clone()],
                }],
                users: vec![User {
                    id: me.clone(),
                    display_name: "me".into(),
                    title: None,
                    presence: Presence::Offline,
                    status: None,
                    availability: Default::default(),
                    is_robot: false,
                    avatar: None,
                }],
            })
        );
        let unreact =
            json!({"ClientMessage": {"Reaction": {"ChatId": "c", "Timestamp": 7, "Type": 100102, "Remove": true}}});
        assert_eq!(
            from_push(&unreact, Some(&me)),
            Some(LiveEvent::Reactions {
                chat_id: ChatId("c".into()),
                ts: 7,
                reactions: vec![],
                users: vec![],
            })
        );
        let seen = json!({"ClientMessage": {"SeenMarker": {"ChatId": "c", "SeqNo": 3}}, "ServerMessageInfo": {"From": {"Guid": "me"}}});
        assert_eq!(
            from_push(&seen, Some(&me)),
            Some(LiveEvent::Seen {
                chat_id: ChatId("c".into())
            })
        );
        let edited = json!({"ClientMessage": {"Plain": {"ChatId": "c", "Text": {"MessageText": "x"}}}, "ServerMessageInfo": {"Timestamp": 7, "LastEditTimestamp": 9, "From": {"Guid": "u1"}}});
        assert!(matches!(from_push(&edited, Some(&me)), Some(LiveEvent::Upsert { message, .. }) if message.edited));
        let heartbeat =
            json!({"ClientMessage": {"Heartbeat": {"Type": 1}}, "ServerMessageInfo": {"From": {"Guid": "u1"}}});
        assert_eq!(
            from_push(&heartbeat, Some(&me)),
            Some(LiveEvent::Online {
                user: UserId("u1".into()),
                online: true
            })
        );
        let status = json!({"ClientMessage": {"UserStatus": {"Availability": 3, "CustomStatus": {"Emoji": "🏖", "Text": "Vacation"}}}, "ServerMessageInfo": {"From": {"Guid": "u1"}}});
        match from_push(&status, Some(&me)) {
            Some(LiveEvent::Status(p)) => {
                assert_eq!(p.availability, Availability::DoNotDisturb);
                assert_eq!(p.status.unwrap().text, "Vacation");
            }
            other => panic!("{other:?}"),
        }
    }
}
