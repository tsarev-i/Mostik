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
    pub muted: bool,
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

/// A call in the chat started or ended, like Slack's huddle messages. `url` is the meeting link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallNote {
    Started { url: String },
    Ended { url: String },
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
        }
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
                CallNote::Started { url } => Some((chat.clone(), url.clone())),
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
}

/// Display name of a chat: the peer's name for a direct chat.
pub fn chat_title(snapshot: &Snapshot, chat: &Chat) -> String {
    match &chat.kind {
        ChatKind::Direct { peer } => snapshot
            .user(peer)
            .map(|u| u.display_name.clone())
            .unwrap_or_else(|| chat.name.clone()),
        _ => chat.name.clone(),
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
}
