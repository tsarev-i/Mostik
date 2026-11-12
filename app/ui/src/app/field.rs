//! The text of a message field: selection offsets, markup around the selection, insertion, and
//! the `@` and `:` suggestions.

use mostik_core::markup::MENTION_ALL;
use mostik_core::*;

use super::state::ctx;

/// Selection of the message field as UTF-16 offsets of its text (see [`crate::editor`]).
pub(crate) fn selection(area: &web_sys::HtmlElement) -> (Option<u32>, Option<u32>) {
    crate::editor::selection(area)
}

/// Sets the text of the message field and puts the caret at UTF-16 offset `caret`.
pub(crate) fn place_caret(area: &web_sys::HtmlElement, value: &str, caret: u32) {
    crate::editor::set_text(area, value, caret, &crate::custom::lookup(ctx()));
}

/// Byte index in `s` of a UTF-16 offset; a missing or out-of-range offset means the end of the text.
/// An offset inside a surrogate pair moves to the end of that character.
pub(crate) fn utf16_to_byte(s: &str, offset: Option<u32>) -> usize {
    let Some(offset) = offset else { return s.len() };
    let mut units = 0;
    for (i, ch) in s.char_indices() {
        if units >= offset as usize {
            return i;
        }
        units += ch.len_utf16();
    }
    s.len()
}

pub(crate) fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// `value` with the `start..end` selection wrapped in `before`/`after`, or unwrapped when it already
/// is (the markers right around it or at its edges), as Slack's formatting buttons toggle.
/// Returns the new value and the selection over the same text.
fn wrap_text(value: &str, start: Option<u32>, end: Option<u32>, before: &str, after: &str) -> (String, u32, u32) {
    let start = utf16_to_byte(value, start);
    let end = utf16_to_byte(value, end).max(start);
    let selected = &value[start..end];
    let (head, tail) = (&value[..start], &value[end..]);
    // The caret at the end of formatted text: the button ends the formatting, the caret moves past the marker.
    if start == end && !after.is_empty() && tail.starts_with(after) && !head.ends_with(before) {
        let caret = utf16_len(head) + utf16_len(after);
        return (value.to_owned(), caret, caret);
    }
    let (head, selected, tail, wrap) = if head.ends_with(before) && tail.starts_with(after) {
        (
            &head[..head.len() - before.len()],
            selected,
            &tail[after.len()..],
            false,
        )
    } else if selected.len() >= before.len() + after.len() && selected.starts_with(before) && selected.ends_with(after)
    {
        (head, &selected[before.len()..selected.len() - after.len()], tail, false)
    } else {
        (head, selected, tail, true)
    };
    let (open, close) = if wrap { (before, after) } else { ("", "") };
    let from = utf16_len(head) + utf16_len(open);
    let to = from + utf16_len(selected);
    (format!("{head}{open}{selected}{close}{tail}"), from, to)
}

/// `value` with `text` in place of the `start..end` selection; `drop_before` characters before the
/// selection are removed too. Returns the new value and the caret after the inserted text.
pub(crate) fn insert_into(
    value: &str,
    start: Option<u32>,
    end: Option<u32>,
    text: &str,
    drop_before: usize,
) -> (String, u32) {
    let end = utf16_to_byte(value, end);
    let selected = utf16_to_byte(value, start).min(end);
    let start = value[..selected]
        .char_indices()
        .rev()
        .take(drop_before)
        .last()
        .map_or(selected, |(i, _)| i);
    let head = format!("{}{text}", &value[..start]);
    let caret = utf16_len(&head);
    (format!("{head}{}", &value[end..]), caret)
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

/// How well `name` matches the lowercase `query`: 0 — starts with it, 1 — one of the words does.
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
    fn formatting_toggles() {
        // Selected text between the markers, the markers selected too, and an empty pair.
        assert_eq!(
            wrap_text("a **b** c", Some(4), Some(5), "**", "**"),
            ("a b c".to_owned(), 2, 3)
        );
        assert_eq!(
            wrap_text("a **b** c", Some(2), Some(7), "**", "**"),
            ("a b c".to_owned(), 2, 3)
        );
        assert_eq!(
            wrap_text("a **** c", Some(4), Some(4), "**", "**"),
            ("a  c".to_owned(), 2, 2)
        );
        assert_eq!(
            wrap_text("a b c", Some(2), Some(3), "~~", "~~"),
            ("a ~~b~~ c".to_owned(), 4, 5)
        );
        // At the end of bold text: the caret leaves it.
        assert_eq!(
            wrap_text("a **b** c", Some(5), Some(5), "**", "**"),
            ("a **b** c".to_owned(), 7, 7)
        );
    }

    #[test]
    fn selection_offsets_are_utf16() {
        // "👍" is two UTF-16 units: "world" spans 9..14.
        let value = "👍 hello world";
        assert_eq!(
            wrap_text(value, Some(9), Some(14), "**", "**"),
            ("👍 hello **world**".to_owned(), 11, 16)
        );
        assert_eq!(
            wrap_text(value, None, None, "`", "`"),
            ("👍 hello world``".to_owned(), 15, 15)
        );
        assert_eq!(
            insert_into(value, Some(2), Some(2), "😀", 0),
            ("👍😀 hello world".to_owned(), 4)
        );
        // `:ok` typed after an emoji: the code and the colon are replaced.
        assert_eq!(
            insert_into("👍 :ok", Some(6), Some(6), "👌", 3),
            ("👍 👌".to_owned(), 5)
        );
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
        assert_eq!(utf16_to_byte("👍x", Some(1)), "👍".len());
    }
}
