//! Custom emoji, as in Slack, kept in the Mostik service chat of an organization.
//!
//! Telemost has no custom emoji, so Mostik clients share them through an ordinary public group,
//! [`SERVICE_CHAT_NAME`], recognised by [`SERVICE_MARKER`] in its description. Mostik hides that
//! chat everywhere; other shared settings may live there later.
//!
//! One emoji is one message of the chat: a single image with the caption `:name:`. Telemost itself
//! vouches for the author and lets only them edit the message, so everyone owns their emoji: a
//! caption edit renames it, deleting the message removes it. Anything else in the chat (plain
//! text, several images, a wrong caption, thread replies) is skipped, so a message typed by hand
//! in the official client cannot break the set. When two emoji share a name, the earlier one wins,
//! as names are global in Slack.
//!
//! Texts carry custom emoji as `:name:`. A custom reaction is a Telemost reaction of Mostik's own
//! type, [`REACTION_BASE`] plus the `SeqNo` of the emoji message: Telemost keeps any 32-bit type,
//! and the number stays the same when the emoji is renamed. Both show only in Mostik.

use serde::{Deserialize, Serialize};

use crate::model::{AttachmentKind, ChatId, Message, Timestamp, UserId};

/// Name of the service chat that Mostik creates in an organization.
pub const SERVICE_CHAT_NAME: &str = "mostik-service";
/// What marks the service chat in its description; the name alone could be anyone's group. `v1`
/// marked the chat `mostik-shared` with the alias `mostik-<org>`, which Mostik no longer uses.
pub const SERVICE_MARKER: &str = "mostik:shared:v2";
/// Description of a new service chat: a note for people who see it in the official client.
pub const SERVICE_DESCRIPTION: &str = "Служебный чат Mostik: общие настройки и кастомные emoji. \
     Пожалуйста, не пишите сюда вручную. / Mostik service chat for shared settings and custom \
     emoji. Please do not post here by hand. mostik:shared:v2";

/// The longest chat alias Telemost takes.
pub const MAX_ALIAS: usize = 20;

/// Alias of the service chat of organization `org`. Telemost resolves an alias for anyone (it is
/// the public link of a chat), while its search finds only the chats the user is already in, so
/// the alias is how colleagues find the chat.
pub fn service_alias(org: u64) -> Option<String> {
    Some(format!("mostiksvc-{org}")).filter(|a| a.len() <= MAX_ALIAS)
}

/// First reaction type of custom emoji ("M" in the high byte), far from Telemost's own types:
/// `1000xx`–`1001xx` and Unicode code points.
pub const REACTION_BASE: u64 = 0x4D00_0000;
/// How many reaction types custom emoji take: one per message number of the service chat.
pub const REACTION_SPAN: u64 = 0x0100_0000;
/// Longest emoji name.
pub const MAX_NAME: usize = 40;
/// Largest image accepted from the chat: a bigger one is skipped. Mostik itself posts at most
/// [`MAX_UPLOAD`], but a person may post by hand.
pub const MAX_BYTES: u64 = 512 * 1024;
/// Largest image Mostik posts, like Slack's limit for custom emoji.
pub const MAX_UPLOAD: usize = 128 * 1024;
/// Side of the square a still image is scaled down into before posting.
pub const UPLOAD_SIDE: u32 = 128;

/// A custom emoji of the service chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomEmoji {
    pub name: String,
    /// Reaction type: [`REACTION_BASE`] + the `SeqNo` of its message.
    pub kind: u64,
    /// The image in the Messenger storage.
    pub file_id: String,
    /// Who posted it: only they can rename or delete it.
    pub owner: UserId,
    /// Time of its message in the service chat.
    pub ts: Timestamp,
}

/// The custom emoji of one organization, from its service chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmojiSet {
    pub chat_id: ChatId,
    /// Organizations of the service chat (Telemost's `OrganizationIds`).
    pub organizations: Vec<u64>,
    /// Emoji in the order they were added.
    pub emoji: Vec<CustomEmoji>,
    /// Emoji whose name an earlier one already has: they do not work until renamed.
    pub taken: Vec<CustomEmoji>,
}

impl EmojiSet {
    /// A set with no emoji yet.
    pub fn empty(chat_id: ChatId, organizations: Vec<u64>) -> Self {
        Self {
            chat_id,
            organizations,
            emoji: vec![],
            taken: vec![],
        }
    }

    /// The emoji called `name`.
    pub fn by_name(&self, name: &str) -> Option<&CustomEmoji> {
        self.emoji.iter().find(|e| e.name == name)
    }

    /// The emoji of reaction type `kind`.
    pub fn by_kind(&self, kind: u64) -> Option<&CustomEmoji> {
        self.emoji.iter().find(|e| e.kind == kind)
    }
}

/// The set for a chat of `organizations` when the user is in `current`: the set of the chat's
/// organization, else the current one's (direct chats belong to no organization), else the
/// only one there is.
pub fn set_for<'a>(sets: &'a [EmojiSet], organizations: &[u64], current: Option<u64>) -> Option<&'a EmojiSet> {
    let of = |org: u64| sets.iter().find(|s| s.organizations.contains(&org));
    organizations
        .iter()
        .find_map(|o| of(*o))
        .or_else(|| current.and_then(of))
        .or_else(|| (sets.len() == 1).then(|| &sets[0]))
}

/// Whether reaction type `kind` is a custom emoji.
pub fn is_custom_reaction(kind: u64) -> bool {
    (REACTION_BASE..REACTION_BASE + REACTION_SPAN).contains(&kind)
}

/// Whether `name` can name a custom emoji: 1–[`MAX_NAME`] lowercase Latin letters, digits, `_`
/// and `-`, and not the shortcode of a standard emoji.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && !is_standard_name(name)
}

/// Whether a standard emoji has the shortcode `name`.
pub fn is_standard_name(name: &str) -> bool {
    emojis::get_by_shortcode(name).is_some()
}

/// A name for an emoji from a file name: lowercase, other characters turned into `_`.
pub fn name_from_file(file: &str) -> String {
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let mut name = String::new();
    for ch in stem.chars().flat_map(char::to_lowercase) {
        let ch = if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' {
            ch
        } else {
            '_'
        };
        if !(ch == '_' && name.ends_with('_')) {
            name.push(ch);
        }
    }
    let name = name.trim_matches('_');
    name.chars().take(MAX_NAME).collect()
}

/// The emoji name in a caption `:name:`.
pub fn caption_name(text: &str) -> Option<&str> {
    let name = text.trim().strip_prefix(':')?.strip_suffix(':')?;
    valid_name(name).then_some(name)
}

/// The caption Mostik gives an emoji message.
pub fn caption(name: &str) -> String {
    format!(":{name}:")
}

/// The custom emoji of a service chat from its messages.
pub fn build(chat_id: &ChatId, organizations: &[u64], messages: &[Message]) -> EmojiSet {
    let mut defs: Vec<(&Message, &str)> = messages
        .iter()
        .filter(|m| &m.chat_id == chat_id && m.thread_parent.is_none() && m.call.is_none())
        .filter(|m| m.forwarded.is_empty() && m.seq_no > 0 && m.seq_no < REACTION_SPAN)
        .filter(|m| match m.attachments.as_slice() {
            [a] => a.kind == AttachmentKind::Image && a.size <= MAX_BYTES,
            _ => false,
        })
        .filter_map(|m| caption_name(&m.text).map(|n| (m, n)))
        .collect();
    defs.sort_by_key(|(m, _)| (m.seq_no, m.ts));
    defs.dedup_by_key(|(m, _)| m.seq_no);
    let mut set = EmojiSet::empty(chat_id.clone(), organizations.to_vec());
    for (m, name) in defs {
        let emoji = CustomEmoji {
            name: name.to_owned(),
            kind: REACTION_BASE + m.seq_no,
            file_id: m.attachments[0].file_id.clone(),
            owner: m.author.clone(),
            ts: m.ts,
        };
        if set.by_name(name).is_some() {
            set.taken.push(emoji);
        } else {
            set.emoji.push(emoji);
        }
    }
    set
}

/// A piece of text with custom emoji: plain text or an emoji name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Emoji(&'a str),
}

/// Splits `text` at the `:name:` codes for which `known` holds.
pub fn split<'a>(text: &'a str, known: impl Fn(&str) -> bool) -> Vec<Piece<'a>> {
    let mut out = Vec::new();
    let mut plain = 0;
    let mut i = 0;
    while let Some(open) = text[i..].find(':').map(|p| i + p) {
        let Some(close) = text[open + 1..].find(':').map(|p| open + 1 + p) else {
            break;
        };
        let name = &text[open + 1..close];
        if !name.is_empty()
            && name.len() <= MAX_NAME
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            && known(name)
        {
            if open > plain {
                out.push(Piece::Text(&text[plain..open]));
            }
            out.push(Piece::Emoji(name));
            plain = close + 1;
            i = close + 1;
        } else {
            // The closing colon may open the next code.
            i = close;
        }
    }
    if plain < text.len() {
        out.push(Piece::Text(&text[plain..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Attachment;

    #[test]
    fn aliases_fit_telemost() {
        assert_eq!(service_alias(1234567).as_deref(), Some("mostiksvc-1234567"));
        assert_eq!(service_alias(9_999_999_999).map(|a| a.len()), Some(MAX_ALIAS));
        assert_eq!(service_alias(u64::MAX), None);
    }

    fn msg(seq: u64, author: &str, text: &str, images: usize) -> Message {
        Message {
            chat_id: ChatId("0/0/s".into()),
            ts: 1000 + seq,
            author: UserId(author.into()),
            text: text.into(),
            thread_parent: None,
            edited: false,
            important: false,
            reactions: vec![],
            thread: None,
            seq_no: seq,
            attachments: (0..images)
                .map(|i| Attachment {
                    kind: AttachmentKind::Image,
                    file_id: format!("file/{seq}-{i}"),
                    name: "a.png".into(),
                    size: 1000,
                    width: 64,
                    height: 64,
                })
                .collect(),
            forwarded: vec![],
            call: None,
        }
    }

    #[test]
    fn names() {
        assert!(valid_name("party_parrot"));
        assert!(valid_name("a-1"));
        assert!(!valid_name(""));
        assert!(!valid_name("Parrot"));
        assert!(!valid_name("no space"));
        assert!(!valid_name("попугай"));
        assert!(!valid_name(&"x".repeat(MAX_NAME + 1)));
        // The standard emoji keep their names: 🦜 is `:parrot:`.
        assert!(!valid_name("parrot"));
        assert!(!valid_name("smile"));
        assert!(!valid_name("+1"));
        assert_eq!(caption_name("  :partyparrot: "), Some("partyparrot"));
        assert_eq!(caption_name(":partyparrot: look"), None);
        assert_eq!(name_from_file("Party Parrot (2).GIF"), "party_parrot_2");
        assert_eq!(name_from_file("ёлка.png"), "");
    }

    #[test]
    fn the_set_skips_anything_else_and_the_first_name_wins() {
        let chat = ChatId("0/0/s".into());
        let mut thread_reply = msg(7, "a", ":reply:", 1);
        thread_reply.thread_parent = Some(1);
        let mut big = msg(8, "a", ":big:", 1);
        big.attachments[0].size = MAX_BYTES + 1;
        let mut other_chat = msg(9, "a", ":other:", 1);
        other_chat.chat_id = ChatId("0/0/x".into());
        let messages = vec![
            msg(5, "b", ":partyparrot:", 1),
            msg(2, "a", ":partyparrot:", 1),
            msg(3, "a", "hello", 0),
            msg(4, "a", ":two:", 2),
            msg(6, "c", "partyparrot", 1),
            msg(10, "c", ":smile:", 1),
            msg(11, "c", ":blob_cat:", 1),
            thread_reply,
            big,
            other_chat,
        ];
        let set = build(&chat, &[42], &messages);
        let names: Vec<(&str, &str)> = set
            .emoji
            .iter()
            .map(|e| (e.name.as_str(), e.owner.0.as_str()))
            .collect();
        assert_eq!(names, [("partyparrot", "a"), ("blob_cat", "c")]);
        assert_eq!(set.taken.len(), 1);
        assert_eq!(set.taken[0].owner.0, "b");
        assert_eq!(set.by_name("partyparrot").map(|e| e.kind), Some(REACTION_BASE + 2));
        assert_eq!(
            set.by_kind(REACTION_BASE + 11).map(|e| e.name.as_str()),
            Some("blob_cat")
        );
        assert_eq!(set.emoji[0].file_id, "file/2-0");
        assert!(is_custom_reaction(REACTION_BASE + 2));
        assert!(!is_custom_reaction(100102));
        assert!(!is_custom_reaction(128512));
    }

    #[test]
    fn set_of_a_chat() {
        let set = |id: &str, org: u64| EmojiSet::empty(ChatId(id.into()), vec![org]);
        let sets = [set("a", 1), set("b", 2)];
        assert_eq!(set_for(&sets, &[2], Some(1)).map(|s| s.chat_id.0.as_str()), Some("b"));
        assert_eq!(set_for(&sets, &[], Some(1)).map(|s| s.chat_id.0.as_str()), Some("a"));
        assert_eq!(set_for(&sets, &[3], None), None);
        assert_eq!(set_for(&sets[..1], &[3], None).map(|s| s.chat_id.0.as_str()), Some("a"));
    }

    #[test]
    fn splits_known_codes() {
        let known = |n: &str| n == "partyparrot" || n == "ok_1";
        assert_eq!(
            split("hi :partyparrot::ok_1: at 10:30:45 :nope: :parrot", known),
            vec![
                Piece::Text("hi "),
                Piece::Emoji("partyparrot"),
                Piece::Emoji("ok_1"),
                Piece::Text(" at 10:30:45 :nope: :parrot"),
            ]
        );
        assert_eq!(
            split("a:b:partyparrot:", known),
            vec![Piece::Text("a:b"), Piece::Emoji("partyparrot")]
        );
        assert_eq!(split("", known), vec![]);
        assert_eq!(split("no codes", known), vec![Piece::Text("no codes")]);
    }
}
