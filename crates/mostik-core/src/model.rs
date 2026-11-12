use serde::{Deserialize, Serialize};

/// Server time of a message in microseconds since the Unix epoch.
/// In Telemost it also identifies the message within its chat.
pub type Timestamp = u64;

/// Messenger user GUID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UserId(pub String);

/// Telemost chat identifier, for example `0/0/<uuid>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChatId(pub String);

impl ChatId {
    /// A chat of Telemost meetings: namespaces 22 and 32 (`namespace_aliases.telemost` in
    /// config.json) and the group call chats of namespace 58.
    pub fn is_meeting(&self) -> bool {
        ["0/22/", "0/32/"].iter().any(|ns| self.0.starts_with(ns)) || self.is_group_call()
    }

    /// The chat Telemost makes for a group call started outside any chat, named like
    /// "Групповой звонок 01.02 в 10:00": namespace 58, `0/58/<uuid>`.
    pub fn is_group_call(&self) -> bool {
        self.0.starts_with("0/58/")
    }
}

/// Coarse presence shown as the dot on an avatar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presence {
    Online,
    Away,
    Offline,
}

/// Availability in Telemost (`UserStatus.Availability`): 0 is the default (online), 1 is away,
/// 2 is busy, 3 is do not disturb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Availability {
    #[default]
    Default,
    Away,
    Busy,
    DoNotDisturb,
}

impl Availability {
    /// Maps the protocol code; unknown codes fall back to the default.
    pub fn from_code(code: u64) -> Self {
        match code {
            1 => Self::Away,
            2 => Self::Busy,
            3 => Self::DoNotDisturb,
            _ => Self::Default,
        }
    }

    /// Protocol code of the availability.
    pub fn code(self) -> u64 {
        match self {
            Self::Default => 0,
            Self::Away => 1,
            Self::Busy => 2,
            Self::DoNotDisturb => 3,
        }
    }
}

/// Presence and status of a person: "online" from the last `Heartbeat` and `UserStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub user: UserId,
    /// `None` means nothing is known about the presence.
    pub online: Option<bool>,
    pub availability: Availability,
    pub status: Option<CustomStatus>,
}

/// Notifications paused on every device of the user: Telemost's "Do not disturb".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pause {
    /// Until when (µs); `None` means until turned back on.
    pub until: Option<Timestamp>,
}

impl Pause {
    /// Whether notifications are still paused at `now` (µs): Telemost does not say when a pause
    /// ends by itself.
    pub fn active_at(&self, now: Timestamp) -> bool {
        self.until.is_none_or(|until| now < until)
    }
}

/// Custom status: an emoji and a short text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomStatus {
    pub emoji: String,
    pub text: String,
    /// Telemost's own "In meeting" status for the time of a call, shown as a call icon
    /// (like the headphones of a Slack huddle) rather than its 📞.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_meeting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub display_name: String,
    /// Job title from the organization directory.
    pub title: Option<String>,
    pub presence: Presence,
    pub status: Option<CustomStatus>,
    #[serde(default)]
    pub availability: Availability,
    pub is_robot: bool,
    /// Yandex avatar identifier (`AvatarId`), if any.
    #[serde(default)]
    pub avatar: Option<String>,
}

/// The name of a person Telemost has said nothing about yet.
pub const UNKNOWN_USER: &str = "Unknown user";

impl User {
    /// Whether the name is a real one and not a stand-in: empty, the GUID or [`UNKNOWN_USER`].
    /// A stand-in gives way to the first real name that comes.
    pub fn has_name(&self) -> bool {
        !self.display_name.is_empty() && self.display_name != self.id.0 && self.display_name != UNKNOWN_USER
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatKind {
    /// Group chat: a channel in Slack terms, public or private.
    Group { public: bool },
    /// Telemost channel: only admins can post.
    Broadcast,
    /// Direct chat with a single peer.
    Direct { peer: UserId },
    /// Chat with oneself ("Notes").
    Notes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    pub id: ChatId,
    pub kind: ChatKind,
    pub name: String,
    pub description: Option<String>,
    pub member_count: u32,
    pub unread: u32,
    pub mentions: u32,
    /// Telemost's `mute`: the chat is quiet in the sidebar and notifies only about mentions,
    /// or about nothing with `mute_mentions` (see [`Chat::notify_level`]).
    pub muted: bool,
    #[serde(default)]
    pub mute_mentions: bool,
    /// Time of the last message, used for sorting.
    pub last_ts: Timestamp,
    /// Pinned message of the chat (`PinnedMessageInfo.PinnedMessageTs`).
    #[serde(default)]
    pub pinned: Option<Timestamp>,
    /// Organizations the chat belongs to (`ChatInfo.OrganizationIds`); empty for direct chats.
    #[serde(default)]
    pub organizations: Vec<u64>,
}

/// User's organization: a "workspace" in Slack terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    pub id: u64,
    pub name: String,
    pub avatar: Option<String>,
}

/// Sidebar section. In Telemost it corresponds to a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub chats: Vec<ChatId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    /// Telemost reaction type (`1001xx`).
    pub kind: u64,
    pub emoji: String,
    pub count: u32,
    pub mine: bool,
    /// Who reacted, as far as Telemost says: `RecentUserReactions` holds only the latest ones,
    /// so the list may be shorter than `count`.
    #[serde(default)]
    pub users: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub reply_count: u32,
    pub last_reply_ts: Timestamp,
    pub participants: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub chat_id: ChatId,
    pub ts: Timestamp,
    pub author: UserId,
    /// Text in Telemost markup: `**bold**`, `__italic__`, `~~strike~~`, `` `code` ``.
    pub text: String,
    /// For a thread reply, the time of the parent message.
    pub thread_parent: Option<Timestamp>,
    pub edited: bool,
    pub important: bool,
    pub reactions: Vec<Reaction>,
    pub thread: Option<ThreadSummary>,
    /// Sequence number within the chat (`SeqNo`), needed to mark messages as read.
    #[serde(default)]
    pub seq_no: u64,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Forwarded messages (`ForwardedMessages`), like a "shared message" in Slack.
    #[serde(default)]
    pub forwarded: Vec<Forwarded>,
    /// Not a text message but Telemost's note that a call in the chat started or ended.
    #[serde(default)]
    pub call: Option<CallNote>,
}

impl Message {
    /// A direct call ringing: a note that only a push brings (history never has it), so it has no
    /// sequence number, must not count as unread and must survive reloading the chat history.
    pub fn is_ringing(&self) -> bool {
        matches!(self.call, Some(CallNote::Ringing { .. }))
    }

    /// The message this one replies to, with the quoted fragment if there is one.
    pub fn reply_to(&self) -> Option<&Forwarded> {
        match self.forwarded.as_slice() {
            [f] if f.reply => Some(f),
            _ => None,
        }
    }
}

/// A call in the chat started or ended, like Slack's huddle messages. `url` is the meeting link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallNote {
    /// The call started. `url` is empty when Telemost does not say: the first call of a group
    /// call chat starts with the chat itself, and nothing in it has the link until the call ends.
    Started { url: String },
    /// Someone is calling the user in a direct chat; it ends with `Ended` like a started call.
    Ringing { url: String },
    Ended {
        url: String,
        /// Why a direct call ended; group calls do not say.
        #[serde(default)]
        reason: Option<CallEndReason>,
        /// How long the call lasted, in seconds: `DurationSeconds` of a direct call. Group calls
        /// do not say; the UI fills it in from the meeting's start when it saw the call end.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seconds: Option<u64>,
    },
}

impl CallNote {
    /// The meeting link.
    pub fn url(&self) -> &str {
        match self {
            CallNote::Started { url } | CallNote::Ringing { url } | CallNote::Ended { url, .. } => url,
        }
    }
}

/// How long a call lasted, in seconds, from its end note: Telemost's own count for a direct call,
/// else the time since `start`, the call's start note or its meeting's creation.
pub fn call_seconds(end: &Message, start: Option<Timestamp>) -> Option<u64> {
    match &end.call {
        Some(CallNote::Ended {
            seconds: Some(seconds), ..
        }) => Some(*seconds),
        Some(CallNote::Ended { .. }) => start
            .filter(|start| *start < end.ts)
            .map(|start| (end.ts - start) / 1_000_000),
        _ => None,
    }
}

/// A call in a chat as Telemost reports it in `StateSync` `meeting_updated`: on every change while
/// it goes on (people join or leave) and once more when it ends, right before its end note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meeting {
    pub chat_id: ChatId,
    /// The meeting link, as in the chat's call notes.
    pub url: String,
    /// When the meeting was created (`create_timestamp`): the start of a group call even when its
    /// start note is not loaded or there is none.
    pub started: Timestamp,
    /// How many people are in the call (`participants_count`).
    pub participants: u32,
    /// The call is over: `status` 2 (0 is "nobody joined yet", 1 "going on").
    pub ended: bool,
}

/// Why a direct call ended: Telemost's `MeetingEndedReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallEndReason {
    /// The person called declined.
    Declined,
    /// The caller hung up before an answer.
    Canceled,
    /// Someone left the call.
    Disconnected,
    /// Nobody answered.
    Timeout,
    /// Everyone left.
    Empty,
    Abandoned,
}

impl CallEndReason {
    /// From the numeric `Reason`.
    pub fn from_code(code: u64) -> Option<Self> {
        Some(match code {
            0 => Self::Declined,
            1 => Self::Canceled,
            2 => Self::Disconnected,
            3 => Self::Timeout,
            4 => Self::Empty,
            5 => Self::Abandoned,
            _ => return None,
        })
    }

    /// The call never connected: declined, cancelled or not answered.
    pub fn unanswered(self) -> bool {
        matches!(self, Self::Declined | Self::Canceled | Self::Timeout)
    }
}

/// A thread the user follows (wrote in, started or was mentioned in), like Slack's "Threads".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowedThread {
    /// The message that started the thread, in its chat.
    pub parent: Message,
    /// Replies the user has not read yet.
    pub unread: u32,
    /// Time and number of the last reply: the order of the list and the read marker.
    pub last_ts: Timestamp,
    pub last_seq: u64,
}

/// Forwarded message: author, time, text and attachments of the original.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forwarded {
    pub author: UserId,
    pub author_name: String,
    pub ts: Timestamp,
    /// Chat of the original, if known.
    pub chat_id: Option<ChatId>,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// The fragment of the original the sender quoted (`ForwardedMessageStyles[].Quote`).
    #[serde(default)]
    pub quote: Option<String>,
    /// The message replies to this one rather than forwards it: Telemost sends a reply as the
    /// only forwarded message from the same chat or thread, or as a quoted one.
    #[serde(default)]
    pub reply: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentKind {
    Image,
    File,
}

/// Message attachment: a gallery image or a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub kind: AttachmentKind,
    /// File identifier in the Messenger storage (`FileInfo.Id2`).
    pub file_id: String,
    pub name: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
}

/// Person's profile from the organization directory (the "Profile" panel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: UserId,
    pub display_name: String,
    pub email: Option<String>,
    pub position: Option<String>,
    pub department: Option<String>,
    pub organization: Option<String>,
    pub nickname: Option<String>,
    pub avatar: Option<String>,
}

/// Chat member with a role (`admin`, `member`, `subscriber`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMember {
    pub profile: UserProfile,
    pub role: String,
}

/// Everything the UI needs for the first render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub me: UserId,
    pub organization: String,
    /// Organization picture (`AvatarId`), like the workspace icon in Slack.
    #[serde(default)]
    pub organization_avatar: Option<String>,
    /// All organizations of the user (`whoami` → `OrganizationInfos`).
    #[serde(default)]
    pub organizations: Vec<Organization>,
    pub users: Vec<User>,
    pub chats: Vec<Chat>,
    pub sections: Vec<Section>,
    pub messages: Vec<Message>,
    /// Telemost chat folders listing their chats by hand, of every organization.
    #[serde(default)]
    pub folders: Vec<ChatFolder>,
    /// Notifications are paused ("Do not disturb").
    #[serde(default)]
    pub paused: Option<Pause>,
}

/// A Telemost chat folder (the `folders` bucket) that lists its chats by hand; Mostik shows it as
/// a sidebar section. Folders by chat type ("Личные", "Новые") are filters and are left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatFolder {
    pub id: String,
    /// Organization id the folder belongs to, as the bucket keys it ("0" without one).
    pub org: String,
    /// The name; it may start with the folder's emoji, as `⚠️|Test` (see [`split_chat_icon`]).
    pub name: String,
    pub chats: Vec<ChatId>,
}

impl Snapshot {
    pub fn user(&self, id: &UserId) -> Option<&User> {
        self.users.iter().find(|u| &u.id == id)
    }

    pub fn chat(&self, id: &ChatId) -> Option<&Chat> {
        self.chats.iter().find(|c| &c.id == id)
    }

    /// Who can be mentioned: known people and the public group chats in the list (the user is in all of them).
    pub fn mention_names(&self) -> crate::markup::MentionNames {
        crate::markup::MentionNames {
            people: self.users.iter().map(|u| u.display_name.clone()).collect(),
            channels: self.mention_channels(),
            ids: self
                .users
                .iter()
                .filter(|u| u.has_name())
                .map(|u| (u.id.0.clone(), u.display_name.clone()))
                .collect(),
        }
    }

    /// Whether `text` mentions the user: by id, by name, through a public chat or with `@all`.
    pub fn mentions_me(&self, text: &str) -> bool {
        let name = self.user(&self.me).map_or("", |u| u.display_name.as_str());
        let me = crate::markup::Me { id: &self.me.0, name };
        self.mention_names()
            .with(|who| crate::markup::mentions_me(text, who, me))
    }

    /// Text without markup for previews, with the names of the people mentioned as `@<guid>`.
    pub fn plain(&self, text: &str) -> String {
        if !text.contains('@') {
            return crate::markup::plain(text, &Default::default());
        }
        let ids: Vec<(&str, &str)> = self
            .users
            .iter()
            .filter(|u| u.has_name())
            .map(|u| (u.id.0.as_str(), u.display_name.as_str()))
            .collect();
        crate::markup::plain(
            text,
            &crate::markup::Mentionable {
                ids: &ids,
                ..Default::default()
            },
        )
    }

    /// Names of the public group chats in the list, which `@<name>` mentions.
    pub fn mention_channels(&self) -> Vec<String> {
        self.chats
            .iter()
            .filter(|c| c.kind == ChatKind::Group { public: true } && !c.name.is_empty())
            .map(|c| c.name.clone())
            .collect()
    }

    /// Chat feed messages without thread replies, oldest first.
    pub fn channel_messages(&self, chat: &ChatId) -> Vec<Message> {
        let mut list: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| &m.chat_id == chat && m.thread_parent.is_none())
            .cloned()
            .collect();
        list.sort_by_key(|m| m.ts);
        list
    }

    /// Calls going on in chats, with their links: the latest call note of a chat among the
    /// loaded messages says the call started and nothing yet says it ended.
    pub fn active_calls(&self) -> std::collections::BTreeMap<ChatId, String> {
        let mut last: std::collections::BTreeMap<&ChatId, (Timestamp, &CallNote)> = Default::default();
        for m in self.messages.iter().filter(|m| m.thread_parent.is_none()) {
            if let Some(note) = &m.call
                && last.get(&m.chat_id).is_none_or(|(ts, _)| m.ts >= *ts)
            {
                last.insert(&m.chat_id, (m.ts, note));
            }
        }
        last.into_iter()
            .filter_map(|(chat, (_, note))| match note {
                CallNote::Started { url } | CallNote::Ringing { url } => Some((chat.clone(), url.clone())),
                CallNote::Ended { .. } => None,
            })
            .collect()
    }

    /// Thread replies, oldest first.
    pub fn thread_replies(&self, chat: &ChatId, parent: Timestamp) -> Vec<Message> {
        let mut list: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| &m.chat_id == chat && m.thread_parent == Some(parent))
            .cloned()
            .collect();
        list.sort_by_key(|m| m.ts);
        list
    }

    /// Authors of the loaded replies in the thread of message `parent`, in the order they
    /// first replied: Telemost does not say who took part in a thread.
    pub fn thread_participants(&self, chat: &ChatId, parent: Timestamp) -> Vec<UserId> {
        let mut out: Vec<UserId> = Vec::new();
        for m in self.thread_replies(chat, parent) {
            if !out.contains(&m.author) {
                out.push(m.author);
            }
        }
        out
    }
}

impl ThreadSummary {
    /// Keeps what this session knows about a thread when Telemost sends the parent again:
    /// Telemost names no participants, and its reply count (the thread's last `SeqNo`) still
    /// counts deleted replies, so a lower known count stands until newer replies come.
    pub fn keep_known(&mut self, known: &ThreadSummary) {
        if self.participants.is_empty() {
            self.participants = known.participants.clone();
        }
        if self.last_reply_ts <= known.last_reply_ts {
            self.reply_count = self.reply_count.min(known.reply_count);
        }
    }
}

/// Notification sounds the preferences offer, in their order there; `none` is silence.
/// The desktop app has a file for each of the others (rendered by `scripts/sounds.py`).
pub const NOTIFICATION_SOUNDS: [&str; 8] = [
    "marimba",
    "chime",
    "drop",
    "knock",
    "glass",
    "electric_piano",
    "retro",
    "none",
];

/// What a chat notifies about, as in Slack: everything, mentions only or nothing. Telemost
/// keeps it in the `chat_mutings` bucket as `{mute, mute_mentions}`; its web client only
/// switches `mute`, which here is "mentions only".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    All,
    Mentions,
    Nothing,
}

impl NotifyLevel {
    /// `(mute, mute_mentions)` for the bucket.
    pub fn flags(self) -> (bool, bool) {
        match self {
            NotifyLevel::All => (false, false),
            NotifyLevel::Mentions => (true, false),
            NotifyLevel::Nothing => (true, true),
        }
    }

    pub fn from_flags(mute: bool, mute_mentions: bool) -> Self {
        match (mute, mute_mentions) {
            (false, _) => NotifyLevel::All,
            (true, false) => NotifyLevel::Mentions,
            (true, true) => NotifyLevel::Nothing,
        }
    }
}

impl Chat {
    pub fn notify_level(&self) -> NotifyLevel {
        NotifyLevel::from_flags(self.muted, self.mute_mentions)
    }

    pub fn set_notify_level(&mut self, level: NotifyLevel) {
        (self.muted, self.mute_mentions) = level.flags();
    }

    /// The emoji a group or channel name starts with (see [`split_chat_icon`]).
    pub fn icon(&self) -> Option<&str> {
        match self.kind {
            ChatKind::Group { .. } | ChatKind::Broadcast => split_chat_icon(&self.name).0,
            ChatKind::Direct { .. } | ChatKind::Notes => None,
        }
    }
}

/// A group or folder name may start with its emoji, as `⚠️|Test`: Telemost has no emoji icons,
/// so Mostik shows the emoji in place of `#`, like the emoji of a section in Slack, and the rest
/// as the name.
pub fn split_chat_icon(name: &str) -> (Option<&str>, &str) {
    if let Some((icon, rest)) = name.split_once('|') {
        let (icon, rest) = (icon.trim(), rest.trim());
        // Emoji only: no letters, digits or ASCII, and short (one emoji, maybe with modifiers).
        let emoji = !icon.is_empty()
            && icon.chars().count() <= 10
            && icon
                .chars()
                .all(|c| !c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace());
        if emoji && !rest.is_empty() {
            return (Some(icon), rest);
        }
    }
    (None, name)
}

/// A group name for plain text, such as a notification: `⚠️ Test` for `⚠️|Test`.
pub fn group_label(name: &str) -> String {
    match split_chat_icon(name) {
        (Some(emoji), rest) => format!("{emoji} {rest}"),
        (None, name) => name.to_owned(),
    }
}

/// Display name of a chat: the peer's name for a direct chat, a group name without its emoji.
pub fn chat_title(snapshot: &Snapshot, chat: &Chat) -> String {
    match &chat.kind {
        ChatKind::Direct { peer } => snapshot
            .user(peer)
            .map(|u| u.display_name.clone())
            .unwrap_or_else(|| chat.name.clone()),
        _ => split_chat_icon(&chat.name).1.to_owned(),
    }
}

/// Slack shows consecutive messages from one author within 5 minutes without an avatar.
pub const GROUPING_WINDOW_MCS: Timestamp = 5 * 60 * 1_000_000;

/// Whether `next` continues the visual group started by `prev`.
/// Call notes stand apart and never join a group.
pub fn continues_group(prev: Option<&Message>, next: &Message) -> bool {
    match prev {
        Some(_) if next.call.is_some() => false,
        Some(p) if p.call.is_some() => false,
        Some(p) => p.author == next.author && next.ts.saturating_sub(p.ts) < GROUPING_WINDOW_MCS,
        None => false,
    }
}

#[cfg(test)]
mod chat_icon_tests {
    use super::*;

    #[test]
    fn group_name_with_emoji() {
        assert_eq!(split_chat_icon("⚠️|Test"), (Some("⚠️"), "Test"));
        assert_eq!(split_chat_icon("🚀 | Релизы"), (Some("🚀"), "Релизы"));
        assert_eq!(split_chat_icon("👩‍💻|dev"), (Some("👩‍💻"), "dev"));
        // Not an emoji, or nothing after it: the name stays whole.
        assert_eq!(split_chat_icon("a|b"), (None, "a|b"));
        assert_eq!(split_chat_icon("Отдел|Продажи"), (None, "Отдел|Продажи"));
        assert_eq!(split_chat_icon("⚠️|"), (None, "⚠️|"));
        assert_eq!(split_chat_icon("plain"), (None, "plain"));
        assert_eq!(group_label("⚠️|Test"), "⚠️ Test");
        assert_eq!(group_label("a|b"), "a|b");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(author: &str, ts: Timestamp) -> Message {
        Message {
            chat_id: ChatId("c".into()),
            ts,
            author: UserId(author.into()),
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
        }
    }

    #[test]
    fn reply_is_the_only_forwarded_message_marked_as_reply() {
        let fwd = |reply: bool| Forwarded {
            author: UserId("u2".into()),
            author_name: String::new(),
            ts: 5,
            chat_id: Some(ChatId("c".into())),
            text: "original".into(),
            attachments: vec![],
            quote: None,
            reply,
        };
        let with = |forwarded| Message {
            forwarded,
            ..msg("u1", 10)
        };
        assert_eq!(with(vec![fwd(true)]).reply_to().map(|f| f.ts), Some(5));
        assert!(with(vec![fwd(false)]).reply_to().is_none());
        assert!(with(vec![fwd(true), fwd(true)]).reply_to().is_none());
        assert!(msg("u1", 10).reply_to().is_none());
    }

    #[test]
    fn call_end_reasons() {
        assert_eq!(CallEndReason::from_code(3), Some(CallEndReason::Timeout));
        assert_eq!(CallEndReason::from_code(9), None);
        assert!(CallEndReason::Timeout.unanswered());
        assert!(CallEndReason::Declined.unanswered());
        assert!(!CallEndReason::Empty.unanswered());
    }

    #[test]
    fn meeting_chats_by_namespace() {
        let id = |s: &str| ChatId(s.into());
        assert!(id("0/22/a").is_meeting() && id("0/32/a").is_meeting());
        assert!(id("0/58/a").is_meeting() && id("0/58/a").is_group_call());
        assert!(!id("0/22/a").is_group_call());
        assert!(!id("0/0/a").is_meeting() && !id("100/58/a_1").is_group_call());
    }

    #[test]
    fn call_lengths() {
        let end = |seconds| Message {
            call: Some(CallNote::Ended {
                url: "u".into(),
                reason: None,
                seconds,
            }),
            ..msg("u1", 100_000_000)
        };
        // Telemost's own count wins over the start.
        assert_eq!(call_seconds(&end(Some(42)), Some(1_000_000)), Some(42));
        assert_eq!(call_seconds(&end(None), Some(40_000_000)), Some(60));
        assert_eq!(call_seconds(&end(None), None), None);
        // A start after the end is someone else's call.
        assert_eq!(call_seconds(&end(None), Some(200_000_000)), None);
        assert_eq!(call_seconds(&msg("u1", 100_000_000), Some(1)), None);
    }

    #[test]
    fn active_calls_follow_the_latest_note() {
        let note = |chat: &str, ts, started: bool| Message {
            chat_id: ChatId(chat.into()),
            call: Some(if started {
                CallNote::Started {
                    url: format!("u-{chat}"),
                }
            } else {
                CallNote::Ended {
                    url: format!("u-{chat}"),
                    reason: None,
                    seconds: None,
                }
            }),
            ..msg("u1", ts)
        };
        let snap = Snapshot {
            messages: vec![
                note("a", 10, true),
                msg("u2", 20),
                note("b", 10, true),
                note("b", 30, false),
                note("c", 30, false),
                note("c", 40, true),
            ],
            me: UserId("u1".into()),
            organization: String::new(),
            organization_avatar: None,
            organizations: vec![],
            users: vec![],
            chats: vec![],
            sections: vec![],
            folders: vec![],
            paused: None,
        };
        let active: Vec<_> = snap.active_calls().into_iter().map(|(c, u)| (c.0, u)).collect();
        assert_eq!(active, [("a".into(), "u-a".into()), ("c".into(), "u-c".into())]);
    }

    #[test]
    fn groups_same_author_within_window() {
        let a = msg("u1", 0);
        let b = msg("u1", GROUPING_WINDOW_MCS - 1);
        assert!(continues_group(Some(&a), &b));
    }

    #[test]
    fn breaks_group_on_author_or_gap() {
        let a = msg("u1", 0);
        assert!(!continues_group(Some(&a), &msg("u2", 10)));
        assert!(!continues_group(Some(&a), &msg("u1", GROUPING_WINDOW_MCS)));
        assert!(!continues_group(None, &a));
    }

    #[test]
    fn mock_snapshot_is_consistent() {
        let s = crate::mock::demo();
        for m in &s.messages {
            assert!(s.chat(&m.chat_id).is_some(), "message in an unknown chat");
            assert!(s.user(&m.author).is_some(), "unknown author");
        }
        for section in &s.sections {
            for id in &section.chats {
                assert!(s.chat(id).is_some(), "section refers to an unknown chat");
            }
        }
    }

    #[test]
    fn thread_summary_keeps_what_the_session_knows() {
        let summary = |count, last, who: &[&str]| ThreadSummary {
            reply_count: count,
            last_reply_ts: last,
            participants: who.iter().map(|w| UserId((*w).into())).collect(),
        };
        // Telemost's count still has a reply deleted in this session; no participants.
        let mut fresh = summary(5, 100, &[]);
        fresh.keep_known(&summary(4, 100, &["a", "b"]));
        assert_eq!(fresh, summary(4, 100, &["a", "b"]));
        // A newer reply came: Telemost's count is taken.
        let mut fresh = summary(6, 120, &[]);
        fresh.keep_known(&summary(4, 100, &["a"]));
        assert_eq!(fresh, summary(6, 120, &["a"]));
    }
}
