//! Mapping of protocol responses to the `mostik-core` domain model.
//!
//! Parsing tolerates missing fields: the protocol is undocumented.
//! The `ChatId` patterns were confirmed on a live session.

use std::collections::BTreeMap;

use mostik_core::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    let mine: Vec<u64> = sm
        .get("RecentUserReactions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| me.is_some_and(|me| s(r, &["UserInfo", "Guid"]) == Some(me.0.as_str())))
        .filter_map(|r| n(r, &["Type"]))
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
            })
        })
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
    Message(Message),
    Deleted {
        chat_id: ChatId,
        ts: Timestamp,
    },
    /// System or service message: never shown in the feed.
    Skip,
}

fn parse_server_message(sm: &Value, chat_hint: Option<&str>, me: Option<&UserId>) -> Parsed {
    let info = &sm["ServerMessageInfo"];
    let Some(plain) = sm.pointer("/ClientMessage/Plain") else {
        return Parsed::Skip;
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
        return Parsed::Deleted { chat_id, ts };
    }
    let thread = n(info, &["ThreadState", "LastSeqNo"])
        .filter(|c| *c > 0)
        .map(|count| ThreadSummary {
            reply_count: count as u32,
            last_reply_ts: n(info, &["ThreadState", "LastTsMcs"]).unwrap_or(ts),
            participants: vec![],
        });
    Parsed::Message(Message {
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
    })
}

/// Forwarded messages: `ForwardedMessages[]{Payload (like Plain), ServerMessageInfo{From, Timestamp}}`.
fn forwarded(sm: &Value) -> Vec<Forwarded> {
    sm.get("ForwardedMessages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let payload = f.get("Payload")?;
            let info = &f["ServerMessageInfo"];
            Some(Forwarded {
                author: UserId(s(info, &["From", "Guid"]).unwrap_or_default().to_owned()),
                author_name: s(info, &["From", "DisplayName"]).unwrap_or_default().to_owned(),
                ts: n(info, &["Timestamp"]).unwrap_or(0),
                chat_id: s(payload, &["ChatId"]).map(|c| ChatId(c.to_owned())),
                text: plain_text(payload),
                attachments: attachments(payload),
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
            if let Parsed::Message(m) = parse_server_message(sm, Some(id), me) {
                last_ts = last_ts.max(m.ts);
                out.messages.push(m);
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
        if let Parsed::Message(m) = parse_server_message(sm, None, me) {
            out.push(m);
        }
    }
    (out, users.into_values().collect())
}

/// UI event built from a Xiva push.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    /// New or changed message; `author` in case the UI does not know the sender.
    Upsert {
        message: Box<Message>,
        author: Option<User>,
    },
    Deleted {
        chat_id: ChatId,
        ts: Timestamp,
    },
    Typing {
        chat_id: ChatId,
        user: UserId,
    },
    /// Resulting reactions of a message after someone changed them.
    Reactions {
        chat_id: ChatId,
        ts: Timestamp,
        reactions: Vec<Reaction>,
    },
    /// The user read the chat (possibly on another device).
    Seen {
        chat_id: ChatId,
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
    /// The connection to Telemost went up or down.
    Connection {
        online: bool,
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
        });
    }
    if let Some(seen) = cm.get("SeenMarker") {
        let chat = s(seen, &["ChatId"])?;
        if me.is_some_and(|me| from.as_ref() == Some(me)) && thread_parent(chat).is_none() {
            return Some(LiveEvent::Seen {
                chat_id: ChatId(chat.to_owned()),
            });
        }
        return None;
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
            message: Box::new(message),
        }),
        Parsed::Deleted { chat_id, ts } => Some(LiveEvent::Deleted { chat_id, ts }),
        Parsed::Skip => None,
    }
}

/// Builds the UI snapshot: "Channels", "Direct messages" and "Meeting chats" sections.
pub fn snapshot(me: User, organization: String, mut data: HistoryData) -> Snapshot {
    data.chats.sort_by(|a, b| b.last_ts.cmp(&a.last_ts));
    let (mut channels, mut dms, mut meetings) = (Vec::new(), Vec::new(), Vec::new());
    // The chat with oneself, like "you" in Slack, always comes first among direct messages.
    if let Some(notes) = data.chats.iter().find(|c| c.kind == ChatKind::Notes) {
        dms.push(notes.id.clone());
    }
    for ch in &data.chats {
        match ch.kind {
            ChatKind::Notes => {}
            ChatKind::Direct { .. } => dms.push(ch.id.clone()),
            _ if is_meeting_chat(&ch.id.0) => meetings.push(ch.id.clone()),
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
                    "Unknown user".into()
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
    }
}

/// `UserStatus` → availability and custom status (emoji and text).
pub fn user_status(v: &Value) -> (Availability, Option<CustomStatus>) {
    let availability = Availability::from_code(n(v, &["Availability"]).unwrap_or(0));
    let status = v.get("CustomStatus").and_then(|c| {
        let emoji = s(c, &["Emoji"]).unwrap_or_default().to_owned();
        let text = s(c, &["Text"]).unwrap_or_default().to_owned();
        (!emoji.is_empty() || !text.is_empty()).then_some(CustomStatus { emoji, text })
    });
    (availability, status)
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

/// How many microseconds after the last `Heartbeat` a person counts as online.
pub const ONLINE_WINDOW_MCS: u64 = 3 * 60 * 1_000_000;

/// "Online" from the `subscribe` response with `MessageBodyType: 2` (last `Heartbeat` and `CurrentTime`).
pub fn online_from_heartbeat(v: &Value) -> Option<bool> {
    let last = n(v, &["LastMessage", "ServerMessageInfo", "Timestamp"])?;
    let now = n(v, &["CurrentTime"])?;
    Some(now.saturating_sub(last) < ONLINE_WINDOW_MCS)
}

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
        last_ts: 0,
        pinned: None,
        organizations: vec![],
    }
}

/// Applies synced settings (`get_buckets`): pinned chats go to the "Starred" section,
/// muted chats set `Chat::muted`.
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
    if let Some(mutings) = bucket("chat_mutings").and_then(|v| v.as_object().cloned()) {
        for ch in &mut snapshot.chats {
            if let Some(m) = mutings.get(&ch.id.0) {
                ch.muted = m.get("mute").and_then(Value::as_bool).unwrap_or(ch.muted);
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
    use super::*;
    use serde_json::json;

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
                mine: true
            }]
        );
        assert_eq!(d.users.len(), 3);
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
        assert_eq!(online_from_heartbeat(&hb(10 * 60_000_000)), Some(false));
        assert_eq!(online_from_heartbeat(&json!({})), None);
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
            {"bucket_name": "chat_mutings", "bucket_value": {"0/0/a": {"mute": true, "mute_mentions": false}}}
        ]}});
        apply_buckets(&mut snap, &buckets);
        assert_eq!(snap.sections[0].id, "starred");
        assert_eq!(snap.sections[0].chats, vec![ChatId("0/0/b".into())]);
        assert!(!snap.sections[1].chats.contains(&ChatId("0/0/b".into())));
        assert!(snap.chat(&ChatId("0/0/a".into())).unwrap().muted);
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
            Some(LiveEvent::Upsert { message, author }) => {
                assert_eq!((message.ts, message.text.as_str(), message.seq_no), (7, "hi", 3));
                assert_eq!(author.unwrap().display_name, "Anna");
            }
            other => panic!("{other:?}"),
        }
        let del = json!({"ClientMessage": {"Plain": {"ChatId": "c", "Timestamp": 7}}, "ServerMessageInfo": {"From": {"Guid": "u1"}, "Timestamp": 7}});
        assert_eq!(
            from_push(&del, Some(&me)),
            Some(LiveEvent::Deleted {
                chat_id: ChatId("c".into()),
                ts: 7
            })
        );
        let del2 = json!({"ClientMessage": {"Plain": {"ChatId": "c", "PayloadId": "p"}}, "ServerMessageInfo": {"Deleted": true, "Timestamp": 8, "From": {"Guid": "u1"}}});
        assert_eq!(
            from_push(&del2, Some(&me)),
            Some(LiveEvent::Deleted {
                chat_id: ChatId("c".into()),
                ts: 8
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
                    mine: true
                }]
            })
        );
        let unreact =
            json!({"ClientMessage": {"Reaction": {"ChatId": "c", "Timestamp": 7, "Type": 100102, "Remove": true}}});
        assert_eq!(
            from_push(&unreact, Some(&me)),
            Some(LiveEvent::Reactions {
                chat_id: ChatId("c".into()),
                ts: 7,
                reactions: vec![]
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
