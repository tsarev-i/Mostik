//! The process contract of Mostik: the Tauri commands and the types the backend and the UI send
//! each other over IPC, command results and event payloads. The live events themselves are
//! domain events, [`mostik_core::event::LiveEvent`]. Both sides use these definitions, so a change
//! of the wire format is a compile error on the other side instead of an event the UI drops with
//! a console warning.
//!
//! The UI and the backend ship together, so no field needs a `#[serde(default)]` for an older peer.
//! The commands themselves, their names, arguments and results, are in [`cmd`].

pub mod cmd;

use serde::{Deserialize, Serialize};

use mostik_core::{Chat, ChatId, FollowedThread, Message, Snapshot, Timestamp, User};

/// Result of `bootstrap` and `login`: either the login screen or a ready snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Boot {
    NeedLogin { reason: Option<String> },
    Ready { snapshot: Box<Snapshot> },
}

/// A history page and the authors of its messages the app has not seen yet: the chat list at
/// start-up holds one message per chat, so most authors of an opened chat arrive only here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    pub users: Vec<User>,
    /// The server may have older messages. Decided by the raw page: service records such as
    /// joins are dropped by the parser, so fewer messages than asked does not mean the end.
    pub more: bool,
    /// The time of the oldest record of the page, deleted messages and service records included:
    /// the next page starts before it, or a run of deleted messages would be asked for again.
    pub oldest: Option<Timestamp>,
    /// How far the others have read the chat (`LastSeenTsMcs`); 0 for a thread.
    pub peer_seen: Timestamp,
}

/// Followed threads and the people they mention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Threads {
    pub threads: Vec<FollowedThread>,
    pub users: Vec<User>,
}

/// Server-side message search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundMessages {
    pub messages: Vec<Message>,
    pub users: Vec<User>,
}

/// A direct chat `open_dm` opened and the person it is with, as the directory knows them: a new
/// chat is not in the list yet, nor may the person be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectChat {
    pub chat: Chat,
    /// The peer, or the user for the chat with oneself. The name is [`mostik_core::UNKNOWN_USER`]
    /// when nobody knows it.
    pub peer: User,
}

/// What "Find a conversation" finds in the organization beyond the user's own chats: public
/// channels and people. The UI leaves out the ones it has already.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryHits {
    /// Public groups, those the user is in among them.
    pub chats: Vec<Chat>,
    /// People with their job titles and photos, the user left out.
    pub people: Vec<User>,
}

/// A chat's name, description, the user's role in it and its member count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatDetails {
    pub name: String,
    pub description: String,
    pub role: String,
    pub member_count: u64,
}

/// A sent message: time and number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    pub ts: Timestamp,
    pub seq_no: u64,
}

/// A file the user picked in the dialog, dropped onto the window or pasted: only such files may
/// be sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub image: bool,
}

/// How far the upload of a file of a sending has gone (`mostik://upload`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadProgress {
    /// The UI's id of the sending ([`cmd::SendFilesArgs::upload`]).
    pub upload: String,
    /// The file, as the UI attached it.
    pub path: String,
    pub sent: u64,
    pub total: u64,
}

/// Files dropped onto the chat window and where, in logical pixels of the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dropped {
    pub files: Vec<PendingFile>,
    pub x: f64,
    pub y: f64,
}

/// The message a clicked notification was about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenTarget {
    pub chat_id: ChatId,
    pub ts: Timestamp,
    pub thread_parent: Option<Timestamp>,
}

/// The call window that is open now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallInfo {
    /// The chat whose meeting it is; `None` for a meeting link from a message.
    pub chat_id: Option<ChatId>,
    /// `#dev`, a person's name or "Telemost meeting".
    pub label: String,
}

/// What the UI shows in the top bar and in Preferences → Updates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// No update known.
    #[default]
    None,
    /// This build cannot update itself: it runs from `cargo` or was not installed from a bundle.
    Unsupported,
    /// A newer release is out: show the "Update" button.
    Available { version: String },
    /// Downloading and installing; `percent` is unknown without a Content-Length.
    Installing { version: String, percent: Option<u8> },
}

/// A call ringing for the user, as the incoming call dialog shows it: only the caller. The
/// backend keeps the ringing ID, the chat and the meeting link to itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ring {
    pub caller: User,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use mostik_core::UserId;

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
    fn boot_round_trips() {
        round_trip(Boot::Ready {
            snapshot: Box::new(mostik_core::mock::demo(1_000_000)),
        });
    }

    #[test]
    fn command_results_keep_their_field_names() {
        pinned(
            Boot::NeedLogin { reason: None },
            json!({ "state": "need_login", "reason": null }),
        );
        pinned(
            UpdateStatus::Installing {
                version: "0.5.0".into(),
                percent: Some(40),
            },
            json!({ "state": "installing", "version": "0.5.0", "percent": 40 }),
        );
        pinned(UpdateStatus::None, json!({ "state": "none" }));
        pinned(
            CallInfo {
                chat_id: Some(chat("c1")),
                label: "#dev".into(),
            },
            json!({ "chatId": "c1", "label": "#dev" }),
        );
        pinned(
            OpenTarget {
                chat_id: chat("c1"),
                ts: 10,
                thread_parent: None,
            },
            json!({ "chat_id": "c1", "ts": 10, "thread_parent": null }),
        );
        pinned(Sent { ts: 10, seq_no: 3 }, json!({ "ts": 10, "seq_no": 3 }));
        pinned(
            UploadProgress {
                upload: "up1".into(),
                path: "/tmp/a.png".into(),
                sent: 5,
                total: 9,
            },
            json!({ "upload": "up1", "path": "/tmp/a.png", "sent": 5, "total": 9 }),
        );
        pinned(
            Dropped {
                files: vec![PendingFile {
                    path: "/tmp/a.png".into(),
                    name: "a.png".into(),
                    size: 5,
                    image: true,
                }],
                x: 1.5,
                y: 2.0,
            },
            json!({
                "files": [{ "path": "/tmp/a.png", "name": "a.png", "size": 5, "image": true }],
                "x": 1.5, "y": 2.0,
            }),
        );
        // Pins only the `caller` key on purpose: `User` itself is pinned in `history_page_and_chat_details`.
        let caller = User::named(UserId("u1".into()), "Ann");
        let caller_json = serde_json::to_value(&caller).unwrap();
        pinned(Ring { caller }, json!({ "caller": caller_json }));
    }

    #[test]
    fn history_page_and_chat_details() {
        pinned(
            HistoryPage {
                messages: vec![],
                users: vec![User::named(UserId("u1".into()), "Ann")],
                more: true,
                oldest: Some(90),
                peer_seen: 80,
            },
            json!({
                "messages": [],
                "users": [{
                    "id": "u1", "display_name": "Ann", "title": null, "presence": "Offline",
                    "status": null, "availability": "Default", "is_robot": false, "avatar": null,
                }],
                "more": true, "oldest": 90, "peer_seen": 80,
            }),
        );
        pinned(
            ChatDetails {
                name: "dev".into(),
                description: "".into(),
                role: "admin".into(),
                member_count: 12,
            },
            json!({ "name": "dev", "description": "", "role": "admin", "member_count": 12 }),
        );
    }

    /// Pins only the keys of these: `Chat`, `Message` and `User` are `mostik-core` types, and
    /// `User` is pinned in `history_page_and_chat_details`.
    #[test]
    fn threads_found_messages_and_direct_chat_keep_their_field_names() {
        let ann = || User::named(UserId("u1".into()), "Ann");
        let ann_json = serde_json::to_value(ann()).unwrap();
        pinned(
            Threads {
                threads: vec![],
                users: vec![ann()],
            },
            json!({ "threads": [], "users": [ann_json.clone()] }),
        );
        pinned(
            FoundMessages {
                messages: vec![],
                users: vec![ann()],
            },
            json!({ "messages": [], "users": [ann_json.clone()] }),
        );
        let chat = mostik_core::Chat::new(chat("u0_u1"), mostik_core::ChatKind::Direct { peer: ann().id });
        let chat_json = serde_json::to_value(&chat).unwrap();
        pinned(
            DirectChat {
                chat: chat.clone(),
                peer: ann(),
            },
            json!({ "chat": chat_json.clone(), "peer": ann_json.clone() }),
        );
        pinned(
            DirectoryHits {
                chats: vec![chat],
                people: vec![ann()],
            },
            json!({ "chats": [chat_json], "people": [ann_json] }),
        );
    }

    #[test]
    fn update_status_and_call_info_edge_cases() {
        pinned(UpdateStatus::Unsupported, json!({ "state": "unsupported" }));
        pinned(
            UpdateStatus::Installing {
                version: "0.5.0".into(),
                percent: None,
            },
            json!({ "state": "installing", "version": "0.5.0", "percent": null }),
        );
        pinned(
            UpdateStatus::Available {
                version: "0.5.0".into(),
            },
            json!({ "state": "available", "version": "0.5.0" }),
        );
        pinned(
            CallInfo {
                chat_id: None,
                label: "Telemost meeting".into(),
            },
            json!({ "chatId": null, "label": "Telemost meeting" }),
        );
    }
}
