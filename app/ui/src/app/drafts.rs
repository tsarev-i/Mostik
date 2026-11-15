//! One store of what is written and not sent yet, for every message field: the composer of a
//! chat, the reply field of a thread and the editor of a message. Each field shows only its own
//! draft, by its key: it fills itself from the store when its key changes and writes back while
//! the user types, so closing a thread or an editor, opening another one or going to another
//! chat loses nothing, as in Slack. The text and the reply bar of a chat or thread are kept
//! between runs; an edit lives while the app runs. The files attached to a field are kept by
//! place as well (`Ctx::pending_files`).

use std::collections::HashMap;

use leptos::prelude::*;
use mostik_core::{ChatId, Timestamp};
use serde::{Deserialize, Serialize};

use crate::ops::ReplyTarget;
use crate::storage;

use super::state::Ctx;

/// The field a draft belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DraftKey {
    /// The composer of a chat.
    Chat(ChatId),
    /// The reply field of the thread of message `parent` of a chat.
    Thread(ChatId, Timestamp),
    /// The editor of message `ts` of a chat, a thread reply's too.
    Edit(ChatId, Timestamp),
}

impl DraftKey {
    /// The key of the composer of `chat`, or of the reply field of its thread `thread`.
    pub fn field(chat: ChatId, thread: Option<Timestamp>) -> Self {
        match thread {
            Some(parent) => Self::Thread(chat, parent),
            None => Self::Chat(chat),
        }
    }

    pub fn chat(&self) -> &ChatId {
        match self {
            Self::Chat(chat) | Self::Thread(chat, _) | Self::Edit(chat, _) => chat,
        }
    }

    /// An edit lives while the app runs; the rest is kept between runs.
    fn kept(&self) -> bool {
        !matches!(self, Self::Edit(..))
    }
}

/// What a field has not sent yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub text: String,
    /// Where the caret was: an editor built again puts it back there.
    pub caret: Option<u32>,
    /// The message the field replies to: the bar above it. Never for an edit.
    pub reply: Option<ReplyTarget>,
}

impl Draft {
    /// Nothing in it at all. Spaces are kept: the field shows what its draft has.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.reply.is_none()
    }

    /// Text to show in "Drafts".
    pub fn has_text(&self) -> bool {
        !self.text.trim().is_empty()
    }
}

pub type Drafts = HashMap<DraftKey, Draft>;

/// The draft of field `key`, empty if there is none.
pub fn get(c: Ctx, key: &DraftKey) -> Draft {
    c.drafts.with_untracked(|d| d.get(key).cloned().unwrap_or_default())
}

/// Writes the text of field `key` and where the caret is; an empty draft goes.
pub fn set_text(c: Ctx, key: &DraftKey, text: &str, caret: Option<u32>) {
    // No draft is the same as an empty one, but for an editor.
    let same = c.drafts.with_untracked(|d| {
        d.get(key)
            .map_or(text.is_empty() && !matches!(key, DraftKey::Edit(..)), |d| {
                d.text == text && d.caret == caret
            })
    });
    if !same {
        update(c, key, |d| {
            d.text = text.to_owned();
            d.caret = caret;
        });
    }
}

/// Starts or ends the reply of field `key`.
pub fn set_reply(c: Ctx, key: &DraftKey, reply: Option<ReplyTarget>) {
    if c.drafts
        .with_untracked(|d| d.get(key).and_then(|d| d.reply.as_ref()) != reply.as_ref())
    {
        update(c, key, |d| d.reply = reply);
    }
}

/// Esc: ends the reply of the field in focus; with the focus outside both fields, of the open
/// chat's composer, or else of the open thread's field. Returns whether there was one.
pub fn cancel_reply(c: Ctx) -> bool {
    let chat = c.current.get_untracked();
    let thread = c.thread.get_untracked().map(|t| DraftKey::Thread(chat.clone(), t));
    let field = document()
        .active_element()
        .and_then(|a| a.closest(".composer-wrap").ok().flatten())
        .map(|wrap| wrap.class_list().contains("in-thread"));
    let keys: Vec<DraftKey> = match field {
        Some(true) => thread.into_iter().collect(),
        Some(false) => vec![DraftKey::Chat(chat)],
        None => std::iter::once(DraftKey::Chat(chat)).chain(thread).collect(),
    };
    let Some(key) = keys
        .into_iter()
        .find(|k| c.drafts.with_untracked(|d| d.get(k).is_some_and(|d| d.reply.is_some())))
    else {
        return false;
    };
    set_reply(c, &key, None);
    true
}

/// Changes the draft of field `key`, dropping it when nothing is left. An editor keeps an empty
/// one: the text emptied on purpose, not the message's own.
fn update(c: Ctx, key: &DraftKey, change: impl FnOnce(&mut Draft)) {
    c.drafts.update(|all| {
        let draft = all.entry(key.clone()).or_default();
        change(draft);
        if draft.is_empty() && !matches!(key, DraftKey::Edit(..)) {
            all.remove(key);
        }
    });
}

/// Drops the draft of field `key`.
pub fn drop_draft(c: Ctx, key: &DraftKey) {
    if c.drafts.with_untracked(|d| d.contains_key(key)) {
        c.drafts.update(|d| {
            d.remove(key);
        });
    }
}

/// The user left chat `chat`: its drafts go.
pub fn forget_chat(c: Ctx, chat: &ChatId) {
    if c.drafts.with_untracked(|d| d.keys().any(|k| k.chat() == chat)) {
        c.drafts.update(|d| d.retain(|k, _| k.chat() != chat));
    }
}

/// Message `ts` of chat `chat` was deleted: the reply field of its thread and its editor go, and
/// so does a reply to it.
pub fn forget_message(c: Ctx, chat: &ChatId, ts: Timestamp) {
    let gone = |k: &DraftKey, d: &Draft| {
        matches!(k, DraftKey::Thread(ch, t) | DraftKey::Edit(ch, t) if ch == chat && *t == ts)
            || d.reply
                .as_ref()
                .is_some_and(|r| &r.msg.chat_id == chat && r.msg.ts == ts)
    };
    if c.drafts.with_untracked(|d| d.iter().any(|(k, v)| gone(k, v))) {
        c.drafts.update(|d| {
            d.retain(|k, _| !matches!(k, DraftKey::Thread(ch, t) | DraftKey::Edit(ch, t) if ch == chat && *t == ts));
            for v in d.values_mut() {
                if v.reply
                    .as_ref()
                    .is_some_and(|r| &r.msg.chat_id == chat && r.msg.ts == ts)
                {
                    v.reply = None;
                }
            }
            d.retain(|_, v| !v.is_empty());
        });
    }
}

/// How many drafts "Drafts" lists: the chats' and threads' with text.
pub fn listed(d: &Drafts) -> usize {
    d.iter()
        .filter(|(k, v)| !matches!(k, DraftKey::Edit(..)) && v.has_text())
        .count()
}

/// Whether chat `chat` has unsent text, in its composer or in one of its threads: the pencil in
/// the sidebar.
pub fn chat_has_text(d: &Drafts, chat: &ChatId) -> bool {
    d.iter()
        .any(|(k, v)| matches!(k, DraftKey::Chat(ch) | DraftKey::Thread(ch, _) if ch == chat) && v.has_text())
}

const DRAFTS_KEY: &str = "mostik.drafts";

/// A draft as it is kept between runs: text and reply, without the caret.
#[derive(Serialize, Deserialize)]
struct Stored {
    text: String,
    reply: Option<ReplyTarget>,
}

/// The drafts kept on this device. Earlier versions kept only the text of chats, as
/// `[[chat, text]]`: those are read too.
pub fn load() -> Drafts {
    if let Some(list) = storage::load::<Vec<(DraftKey, Stored)>>(DRAFTS_KEY) {
        return list
            .into_iter()
            .filter(|(k, _)| k.kept())
            .map(|(k, s)| {
                (
                    k,
                    Draft {
                        text: s.text,
                        caret: None,
                        reply: s.reply,
                    },
                )
            })
            .filter(|(_, d)| !d.is_empty())
            .collect();
    }
    storage::load::<Vec<(ChatId, String)>>(DRAFTS_KEY)
        .unwrap_or_default()
        .into_iter()
        .map(|(chat, text)| {
            (
                DraftKey::Chat(chat),
                Draft {
                    text,
                    ..Draft::default()
                },
            )
        })
        .filter(|(_, d)| !d.is_empty())
        .collect()
}

/// Keeps the drafts of chats and threads: their text and reply.
pub fn save(d: &Drafts) {
    let list: Vec<(&DraftKey, Stored)> = d
        .iter()
        .filter(|(k, v)| k.kept() && !v.is_empty())
        .map(|(k, v)| {
            (
                k,
                Stored {
                    text: v.text.clone(),
                    reply: v.reply.clone(),
                },
            )
        })
        .collect();
    storage::save(DRAFTS_KEY, &list);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_text() {
        let chat = ChatId("c".into());
        assert_eq!(DraftKey::field(chat.clone(), None), DraftKey::Chat(chat.clone()));
        assert_eq!(
            DraftKey::field(chat.clone(), Some(5)),
            DraftKey::Thread(chat.clone(), 5)
        );
        assert!(!DraftKey::Edit(chat.clone(), 5).kept());
        let mut d = Drafts::new();
        assert!(!chat_has_text(&d, &chat));
        d.insert(
            DraftKey::Thread(chat.clone(), 5),
            Draft {
                text: "hi".into(),
                ..Draft::default()
            },
        );
        assert!(chat_has_text(&d, &chat));
        assert!(!chat_has_text(&d, &ChatId("other".into())));
        // Spaces are kept for the field, but listed nowhere.
        let spaces = Draft {
            text: "  ".into(),
            ..Draft::default()
        };
        assert!(!spaces.is_empty() && !spaces.has_text());
        assert!(Draft::default().is_empty());
    }

    /// The format kept between runs: keys tagged by kind, as a list of pairs.
    #[test]
    fn stored_format() {
        let chat = ChatId("0/0/a".into());
        let list = vec![(
            DraftKey::Thread(chat, 7),
            Stored {
                text: "x".into(),
                reply: None,
            },
        )];
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            serde_json::json!([[{"Thread": ["0/0/a", 7]}, {"text": "x", "reply": null}]])
        );
    }
}
