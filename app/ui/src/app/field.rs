//! The text of a message field in the DOM: its selection, markup around the selection, insertion,
//! and the `@` and `:` suggestions. What these do to the text is in [`crate::text_edit`].

use mostik_core::markup::MENTION_ALL;
use mostik_core::*;

use super::state::ctx;
use crate::text_edit::{insert_into, utf16_to_byte, wrap_text};

/// Selection of the message field as UTF-16 offsets of its text (see [`crate::editor`]).
pub(crate) fn selection(area: &web_sys::HtmlElement) -> (Option<u32>, Option<u32>) {
    crate::editor::selection(area)
}

/// Sets the text of the message field and puts the caret at UTF-16 offset `caret`.
pub(crate) fn place_caret(area: &web_sys::HtmlElement, value: &str, caret: u32) {
    crate::editor::set_text(area, value, caret, &crate::custom::lookup(ctx()));
}

/// `:code` that ends at UTF-16 offset `caret` (two characters or more), for emoji suggestions.
fn emoji_code_before(value: &str, caret: u32) -> Option<String> {
    let before = &value[..utf16_to_byte(value, Some(caret))];
    let colon = before.rfind(':')?;
    let code = &before[colon + 1..];
    let starts_word = before[..colon].chars().next_back().is_none_or(char::is_whitespace);
    (starts_word
        && code.chars().count() >= 2
        && code.chars().all(|ch| ch.is_ascii_alphanumeric() || "_+-".contains(ch)))
    .then(|| code.to_owned())
}

/// Wraps the composer selection in Telemost markup markers.
pub(crate) fn wrap_selection(area: &web_sys::HtmlElement, before: &str, after: &str) -> String {
    let (start, end) = selection(area);
    let (value, from, to) = wrap_text(&crate::editor::value(area), start, end, before, after);
    place_caret(area, &value, to);
    crate::editor::set_selection(area, from, to);
    value
}

/// Inserts `text` in place of the selection (or at the cursor); `drop_before` is how many characters before the cursor to remove
/// (the typed `:code` when picking a suggestion).
pub(super) fn insert_text(area: &web_sys::HtmlElement, text: &str, drop_before: usize) -> String {
    let (start, end) = selection(area);
    let (value, caret) = insert_into(&crate::editor::value(area), start, end, text, drop_before);
    place_caret(area, &value, caret);
    value
}

/// The name typed after `@` that ends at UTF-16 offset `caret`, for mention suggestions; may be empty.
/// Names may contain spaces, so the query runs up to three words.
fn mention_query_before(value: &str, caret: u32) -> Option<String> {
    let before = &value[..utf16_to_byte(value, Some(caret))];
    let at = before.rfind('@')?;
    let query = &before[at + 1..];
    let starts_word = before[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
    (starts_word
        && query.chars().count() <= 40
        && query.split(' ').count() <= 3
        && !query.starts_with(' ')
        && !query.contains("  ")
        && !query.contains(['\n', '@']))
    .then(|| query.to_owned())
}

/// `@name` before the cursor, for mention suggestions.
pub(super) fn mention_query(area: &web_sys::HtmlElement) -> Option<String> {
    let caret = selection(area).0?;
    mention_query_before(&crate::editor::value(area), caret)
}

/// What the composer suggests while typing: an emoji after `:` or a mention after `@`.
#[derive(Clone)]
pub(super) enum Suggestion {
    Emoji {
        glyph: &'static str,
        code: String,
    },
    /// A custom emoji of the organization: inserted as its `:name:`.
    Custom(mostik_core::custom_emoji::CustomEmoji),
    Person(User),
    /// Public group chat: mentions its members.
    Channel(String),
    /// `@all`: everyone in the chat.
    All,
}

/// How well `name` matches the lowercase `query`: 0 if it starts with it, 1 if one of its words does.
fn mention_rank(name: &str, query: &str) -> Option<u8> {
    let name = name.to_lowercase();
    if name.starts_with(query) {
        Some(0)
    } else {
        name.split_whitespace().any(|w| w.starts_with(query)).then_some(1)
    }
}

/// Mention suggestions for `query` in `chat`: `@all` (not in direct chats), then people, then public chats.
pub(super) fn mention_suggestions(s: &Snapshot, chat: &ChatId, query: &str, limit: usize) -> Vec<Suggestion> {
    let q = query.to_lowercase();
    let mut out = vec![];
    let group = s
        .chat(chat)
        .is_some_and(|ch| matches!(ch.kind, ChatKind::Group { .. } | ChatKind::Broadcast));
    if group && MENTION_ALL.starts_with(&q) {
        out.push(Suggestion::All);
    }
    let mut people: Vec<(u8, &User)> = s
        .users
        .iter()
        .filter(|u| u.id != s.me && !u.display_name.is_empty())
        .filter_map(|u| mention_rank(&u.display_name, &q).map(|r| (r, u)))
        .collect();
    people.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.display_name.cmp(&b.1.display_name)));
    let mut channels: Vec<(u8, String)> = s
        .mention_channels()
        .into_iter()
        .filter_map(|n| mention_rank(&n, &q).map(|r| (r, n)))
        .collect();
    channels.sort();
    channels.dedup();
    // Keep room for chats when many people match.
    let room = limit.saturating_sub(out.len());
    let take_people = people.len().min(room.saturating_sub(channels.len().min(2)));
    out.extend(
        people
            .into_iter()
            .take(take_people)
            .map(|(_, u)| Suggestion::Person(u.clone())),
    );
    let room = limit.saturating_sub(out.len());
    out.extend(channels.into_iter().take(room).map(|(_, n)| Suggestion::Channel(n)));
    out
}

/// `:code` before the cursor (two characters or more), for emoji suggestions, as in Slack.
pub(super) fn emoji_query(area: &web_sys::HtmlElement) -> Option<String> {
    let caret = selection(area).0?;
    emoji_code_before(&crate::editor::value(area), caret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_code_runs_to_the_cursor() {
        assert_eq!(emoji_code_before("👍 :ok", 6).as_deref(), Some("ok"));
        assert_eq!(emoji_code_before("a:ok", 4), None);
    }

    #[test]
    fn mention_query_runs_to_the_cursor() {
        assert_eq!(mention_query_before("hi @", 4).as_deref(), Some(""));
        assert_eq!(mention_query_before("hi @Ivan Pe", 11).as_deref(), Some("Ivan Pe"));
        assert_eq!(mention_query_before("(@back", 6).as_deref(), Some("back"));
        assert_eq!(mention_query_before("mail a@b", 8), None);
        assert_eq!(mention_query_before("@one two three four", 19), None);
        assert_eq!(mention_query_before("@a\nb", 4), None);
    }

    #[test]
    fn mention_suggestions_order() {
        let mut s = mostik_core::mock::demo(1_794_300_000_000_000);
        let general = s
            .chats
            .iter()
            .find(|ch| ch.kind == ChatKind::Group { public: true })
            .unwrap()
            .clone();
        let picks = mention_suggestions(&s, &general.id, "", 50);
        assert!(matches!(picks.first(), Some(Suggestion::All)));
        assert!(
            picks
                .iter()
                .any(|p| matches!(p, Suggestion::Channel(n) if *n == general.name))
        );
        assert!(!picks.iter().any(|p| matches!(p, Suggestion::Person(u) if u.id == s.me)));
        // No `@all` in direct chats.
        let direct = s
            .chats
            .iter()
            .find(|ch| matches!(ch.kind, ChatKind::Direct { .. }))
            .unwrap()
            .id
            .clone();
        assert!(
            !mention_suggestions(&s, &direct, "", 50)
                .iter()
                .any(|p| matches!(p, Suggestion::All))
        );
        // Private groups are not offered.
        s.chats.retain(|ch| ch.kind != ChatKind::Group { public: true });
        assert!(
            !mention_suggestions(&s, &general.id, "", 50)
                .iter()
                .any(|p| matches!(p, Suggestion::Channel(_)))
        );
        assert_eq!(mention_rank("Ivan Petrov", "pe"), Some(1));
        assert_eq!(mention_rank("Ivan Petrov", "iv"), Some(0));
        assert_eq!(mention_rank("Ivan Petrov", "x"), None);
    }
}
