//! Live events: what changed in the chats, as the backend learns it from Telemost pushes
//! (`mostik_yandex::convert::from_push`) or makes it itself, and sends to the UI.

use serde::{Deserialize, Serialize};

use crate::{
    Chat, ChatFolder, ChatId, FollowedThread, Meeting, Message, NotifyLevel, Pause, PresenceInfo, Reaction, Reads,
    Timestamp, User, UserId,
};

/// How the sender of a message wants the members told of it (`NotificationBehaviour`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alert {
    /// As each member's settings say.
    #[default]
    Default,
    /// A notification without a sound (`MUTED`): "Without sound" by the field.
    Quiet,
    /// No notification at all (`SILENT`): the later parts of a long message.
    Silent,
}

impl Alert {
    /// From `NotificationBehaviour`: `SILENT` 1, `MUTED` 3; the rest (`DEFAULT` 0, `IGNORE_MUTES`
    /// 2) notify as usual.
    pub fn from_code(code: u64) -> Self {
        match code {
            1 => Self::Silent,
            3 => Self::Quiet,
            _ => Self::Default,
        }
    }
}

/// A live event, built from a Telemost push (`mostik_yandex::convert::from_push`) or by the
/// backend itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    /// New or changed message; `author` in case the UI does not know the sender, `mentioned`
    /// for the names of the people it mentions.
    Upsert {
        message: Box<Message>,
        author: Option<User>,
        mentioned: Vec<User>,
        /// How the sender wants the members told of it.
        #[serde(default)]
        alert: Alert,
    },
    /// A message was deleted; `thread_parent` is set for a thread reply.
    Deleted {
        chat_id: ChatId,
        ts: Timestamp,
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
        users: Vec<User>,
    },
    /// The user read the chat (possibly on another device).
    Seen {
        chat_id: ChatId,
    },
    /// Someone else read the chat up to message `ts`; `reads` are that message's counters, if
    /// it is the user's.
    PeerSeen {
        chat_id: ChatId,
        user: UserId,
        ts: Timestamp,
        reads: Option<Reads>,
    },
    /// The user read the thread of message `parent` in `chat_id` (possibly on another device).
    ThreadSeen {
        chat_id: ChatId,
        parent: Timestamp,
    },
    /// A person came online (their `Heartbeat` arrived).
    Online {
        user: UserId,
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
    /// The user is in a new chat: created it, was added to it, or someone started a direct chat.
    /// For the backend: it normally swallows this one, fetches the chat and sends
    /// [`LiveEvent::ChatAdded`] instead. The UI ignores it if it gets through.
    Joined {
        chat_id: ChatId,
    },
    /// The user left the chat or was removed from it.
    ChatRemoved {
        chat_id: ChatId,
    },
    /// The chat's name, description or member count changed; `None` is "not reported".
    ChatChanged {
        chat_id: ChatId,
        name: Option<String>,
        description: Option<String>,
        member_count: Option<u32>,
    },
    /// The chat folders of every organization, after a change on any device.
    Folders {
        folders: Vec<ChatFolder>,
    },
    /// The pinned ("starred") chats, after a change on any device.
    Starred {
        chats: Vec<ChatId>,
    },
    /// What every chat notifies about, after a change on any device: chats not listed notify
    /// about everything.
    Mutings {
        chats: Vec<(ChatId, NotifyLevel)>,
    },
    /// A call in a chat changed: people joined or left, or it ended.
    Meeting(Meeting),
    /// Notifications were paused or resumed ("Do not disturb"), on any device.
    Paused {
        pause: Option<Pause>,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::{Availability, CustomStatus};

    /// Serializes `value`, checks the JSON against `expected` and reads it back unchanged.
    fn pinned<T>(value: T, expected: Value)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(json, expected);
        assert_eq!(serde_json::from_value::<T>(json).unwrap(), value);
    }

    /// Serializes `value` and reads it back unchanged.
    fn round_trip<T>(value: T) -> Value
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(serde_json::from_value::<T>(json.clone()).unwrap(), value);
        json
    }

    fn chat(id: &str) -> ChatId {
        ChatId(id.to_owned())
    }

    #[test]
    fn live_events_are_tagged_by_type_in_snake_case() {
        pinned(
            LiveEvent::Deleted {
                chat_id: chat("c1"),
                ts: 10,
                thread_parent: Some(5),
            },
            json!({ "type": "deleted", "chat_id": "c1", "ts": 10, "thread_parent": 5 }),
        );
        pinned(
            LiveEvent::Typing {
                chat_id: chat("c1"),
                user: UserId("u1".into()),
            },
            json!({ "type": "typing", "chat_id": "c1", "user": "u1" }),
        );
        pinned(
            LiveEvent::PeerSeen {
                chat_id: chat("c1"),
                user: UserId("u1".into()),
                ts: 7,
                reads: None,
            },
            json!({ "type": "peer_seen", "chat_id": "c1", "user": "u1", "ts": 7, "reads": null }),
        );
        pinned(
            LiveEvent::ChatChanged {
                chat_id: chat("c1"),
                name: Some("dev".into()),
                description: None,
                member_count: Some(3),
            },
            json!({
                "type": "chat_changed", "chat_id": "c1", "name": "dev", "description": null,
                "member_count": 3,
            }),
        );
        pinned(
            LiveEvent::Connection { online: true },
            json!({ "type": "connection", "online": true }),
        );
    }

    #[test]
    fn live_events_with_core_types_round_trip() {
        let snapshot = crate::mock::demo(1_000_000);
        let message = snapshot.messages[0].clone();
        let author = snapshot.users.first().cloned();
        let json = round_trip(LiveEvent::Upsert {
            message: Box::new(message),
            author,
            mentioned: vec![],
            alert: Alert::Quiet,
        });
        assert_eq!(json["type"], "upsert");
        round_trip(LiveEvent::Mutings {
            chats: vec![(chat("c1"), NotifyLevel::Mentions)],
        });
        let json = round_trip(LiveEvent::Status(PresenceInfo {
            user: UserId("u1".into()),
            online: Some(true),
            availability: Availability::Away,
            status: None,
        }));
        // A newtype variant carries the tag among the fields of its content.
        assert_eq!(json["type"], "status");
        assert_eq!(json["user"], "u1");
    }

    #[test]
    fn live_events_with_core_payloads_are_pinned() {
        pinned(
            LiveEvent::Mutings {
                chats: vec![(chat("c1"), NotifyLevel::Mentions), (chat("c2"), NotifyLevel::Nothing)],
            },
            json!({ "type": "mutings", "chats": [["c1", "mentions"], ["c2", "nothing"]] }),
        );
        // Newtype variants: the fields of the content sit next to the `type` tag.
        pinned(
            LiveEvent::Status(PresenceInfo {
                user: UserId("u1".into()),
                online: Some(true),
                availability: Availability::DoNotDisturb,
                status: Some(CustomStatus {
                    emoji: "🌴".into(),
                    text: "Vacation".into(),
                    in_meeting: false,
                }),
            }),
            json!({
                "type": "status", "user": "u1", "online": true, "availability": "DoNotDisturb",
                "status": { "emoji": "🌴", "text": "Vacation" },
            }),
        );
        pinned(
            LiveEvent::Meeting(Meeting {
                chat_id: chat("c1"),
                url: "https://telemost.yandex.ru/j/1".into(),
                started: 100,
                participants: 2,
                ended: false,
            }),
            json!({
                "type": "meeting", "chat_id": "c1", "url": "https://telemost.yandex.ru/j/1",
                "started": 100, "participants": 2, "ended": false,
            }),
        );
        pinned(
            LiveEvent::Paused { pause: None },
            json!({ "type": "paused", "pause": null }),
        );
        pinned(
            LiveEvent::Paused {
                pause: Some(Pause { until: Some(500) }),
            },
            json!({ "type": "paused", "pause": { "until": 500 } }),
        );
        pinned(
            LiveEvent::Pinned {
                chat_id: chat("c1"),
                ts: None,
            },
            json!({ "type": "pinned", "chat_id": "c1", "ts": null }),
        );
    }
}
