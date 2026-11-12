//! Chats hidden from the sidebar with "×", as closing a conversation in Slack. Kept in
//! `localStorage`: chat → time of its last message when it was hidden. A hidden chat comes back
//! with a new message (a muted one only with a mention) or when it is opened.

use std::collections::HashMap;

use mostik_core::{Chat, ChatId, Timestamp};

pub type Hidden = HashMap<ChatId, Timestamp>;

const KEY: &str = "mostik.hidden";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn load() -> Hidden {
    storage()
        .and_then(|s| s.get_item(KEY).ok().flatten())
        .and_then(|v| serde_json::from_str::<Vec<(ChatId, Timestamp)>>(&v).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

pub fn save(hidden: &Hidden) {
    if let (Some(s), Ok(v)) = (storage(), serde_json::to_string(&hidden.iter().collect::<Vec<_>>())) {
        let _ = s.set_item(KEY, &v);
    }
}

/// Whether the chat, hidden when its last message was at `at`, has something new to show again.
pub fn came_back(ch: &Chat, at: Timestamp) -> bool {
    ch.mentions > 0 || (ch.last_ts > at && !ch.muted)
}

/// Whether the sidebar hides the chat.
pub fn hides(hidden: &Hidden, ch: &Chat) -> bool {
    hidden.get(&ch.id).is_some_and(|&at| !came_back(ch, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mostik_core::ChatKind;

    fn chat(last_ts: Timestamp, muted: bool, mentions: u32) -> Chat {
        Chat {
            id: ChatId("c".into()),
            kind: ChatKind::Group { public: true },
            name: "general".into(),
            description: None,
            member_count: 2,
            unread: 0,
            mentions,
            muted,
            mute_mentions: false,
            last_ts,
            peer_seen_ts: 0,
            pinned: None,
            organizations: vec![],
        }
    }

    #[test]
    fn hidden_chat_comes_back_with_news() {
        let hidden = Hidden::from([(ChatId("c".into()), 10)]);
        assert!(hides(&hidden, &chat(10, false, 0)));
        assert!(!hides(&hidden, &chat(11, false, 0)));
        // A muted chat waits for a mention.
        assert!(hides(&hidden, &chat(11, true, 0)));
        assert!(!hides(&hidden, &chat(11, true, 1)));
        assert!(!hides(&Hidden::new(), &chat(10, false, 0)));
    }
}
