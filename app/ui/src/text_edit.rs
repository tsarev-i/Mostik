//! What typing, deleting, formatting, inserting and editing links do to the text of a message
//! field: pure functions of its text, in Telemost markup, and of UTF-16 offsets into it, the units
//! the field's selection has. The DOM of the field is in [`crate::editor`], the component that
//! wires it in [`crate::compose`].

use std::ops::Range;

use mostik_core::markup::{EditSpan, Mark, edit_spans, link_address, link_at, link_markup, pasted_link, typed_address};

/// Byte index in `s` of a UTF-16 offset; a missing or out-of-range offset means the end of the text.
/// An offset inside a surrogate pair moves to the end of that character.
pub fn utf16_to_byte(s: &str, offset: Option<u32>) -> usize {
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

/// The length of `s` in UTF-16 units.
pub fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// `value` with the `start..end` selection wrapped in `before`/`after`, or unwrapped when it already
/// is (the markers right around it or at its edges), as Slack's formatting buttons toggle.
/// Returns the new value and the selection over the same text.
pub(crate) fn wrap_text(
    value: &str,
    start: Option<u32>,
    end: Option<u32>,
    before: &str,
    after: &str,
) -> (String, u32, u32) {
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

/// The marked spans of the field's text as byte offsets: where each one starts, where its text
/// starts and ends, and where it ends. Only spans with markers: a bare address is edited as text.
pub(crate) fn marked_spans(text: &str) -> Vec<[usize; 4]> {
    let mut out = Vec::new();
    let mut at = 0;
    for span in edit_spans(text) {
        match span {
            EditSpan::Plain(t) => at += t.len(),
            EditSpan::Marked { open, inner, close, .. } => {
                let inner_at = at + open.len();
                let close_at = inner_at + inner.len();
                if !(open.is_empty() && close.is_empty()) {
                    out.push([at, inner_at, close_at, close_at + close.len()]);
                }
                at = close_at + close.len();
            }
        }
    }
    out
}

/// `text` with UTF-16 offsets `start..end` deleted as the field shows them: the shown text goes,
/// the hidden markers stay, and a span whose text goes whole goes with its markers. Returns the
/// text and the caret; `None` when no marker is in or next to the range, so the browser can delete
/// it itself: it would take a hidden marker next to the range along, as the same caret position.
pub(crate) fn delete_shown(text: &str, start: u32, end: u32) -> Option<(String, u32)> {
    let from = utf16_to_byte(text, Some(start));
    let to = utf16_to_byte(text, Some(end)).max(from);
    let spans = marked_spans(text);
    let near = |a: usize, b: usize| a <= to && from <= b;
    if !spans
        .iter()
        .any(|&[open, inner, close, end]| near(open, inner) || near(close, end))
    {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut caret = 0;
    for (i, ch) in text.char_indices() {
        let gone = match spans.iter().find(|&&[open, _, _, end]| open <= i && i < end) {
            Some(&[_, inner, close, _]) if inner < close && from <= inner && close <= to => true,
            Some(&[_, inner, close, _]) if i < inner || i >= close => false,
            _ => from <= i && i < to,
        };
        if !gone {
            out.push(ch);
            if i < from {
                caret += ch.len_utf16() as u32;
            }
        }
    }
    Some((out, caret))
}

/// UTF-16 offset `at` of the field's text moved back or forth over the hidden markers next to it.
pub(crate) fn past_markers(text: &str, at: u32, back: bool) -> u32 {
    let spans = marked_spans(text);
    let mut at = utf16_to_byte(text, Some(at));
    while let Some(next) = spans.iter().find_map(|&[open, inner, close, end]| match back {
        true if at == inner => Some(open),
        true if at == end => Some(close),
        false if at == open => Some(inner),
        false if at == close => Some(end),
        _ => None,
    }) {
        at = next;
    }
    utf16_len(&text[..at])
}

/// `text` with `data` typed at UTF-16 offset `at`, when `at` is where a link's caption starts or
/// ends inside its hidden markers: the typed text goes outside the link, before `[` or after
/// `](address)`, as it does at the end of a line. (Bold and the like go on as typed, until their
/// hotkey ends them.) Returns the text and the caret; `None` elsewhere, where the browser types.
pub(crate) fn insert_shown(text: &str, at: u32, data: &str) -> Option<(String, u32)> {
    let byte = utf16_to_byte(text, Some(at));
    let mut from = 0;
    let mut to = None;
    for span in edit_spans(text) {
        match span {
            EditSpan::Plain(t) => from += t.len(),
            EditSpan::Marked {
                kind,
                open,
                inner,
                close,
            } => {
                let (inner_at, close_at) = (from + open.len(), from + open.len() + inner.len());
                let end = close_at + close.len();
                if kind == Mark::Link && byte == close_at {
                    to = Some(end);
                } else if kind == Mark::Link && byte == inner_at {
                    to = Some(from);
                }
                from = end;
            }
        }
    }
    let to = to?;
    let out = format!("{}{data}{}", &text[..to], &text[to..]);
    Some((out, utf16_len(&text[..to]) + utf16_len(data)))
}

/// What the link dialog edits: the link the selection is in, or the selected text to make a link of.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LinkTarget {
    /// The text of the field when the dialog opened, and the selection in it.
    pub(crate) text: String,
    pub(crate) selection: (u32, u32),
    /// Bytes of `text` the link takes the place of.
    pub(crate) range: Range<usize>,
    pub(crate) caption: String,
    pub(crate) address: String,
    /// An existing link, which the dialog can also remove.
    pub(crate) existing: bool,
}

impl LinkTarget {
    pub(crate) fn new(text: &str, start: Option<u32>, end: Option<u32>) -> Self {
        let from = utf16_to_byte(text, start);
        let to = utf16_to_byte(text, end).max(from);
        let selection = (utf16_len(&text[..from]), utf16_len(&text[..to]));
        let (range, caption, address, existing) = match link_at(text, from, to) {
            Some(link) => (
                link.range,
                text[link.caption].to_owned(),
                typed_address(&link.address),
                true,
            ),
            // A caption is one line.
            None => (from..to, text[from..to].replace('\n', " "), String::new(), false),
        };
        Self {
            text: text.to_owned(),
            selection,
            range,
            caption,
            address,
            existing,
        }
    }

    /// The text with the link saved, and the caret after it; `None` when the address is not a
    /// web or mail address or the caption cannot be one. Without a caption the address stays bare.
    pub(crate) fn save(&self, caption: &str, address: &str) -> Option<(String, u32)> {
        let address = link_address(address)?;
        if caption.trim().is_empty() {
            return Some(self.replace(&address));
        }
        Some(self.replace(&link_markup(caption, &address)?))
    }

    /// The text with the link taken off: its caption stays as plain text.
    pub(crate) fn remove(&self) -> (String, u32) {
        self.replace(&self.caption)
    }

    fn replace(&self, with: &str) -> (String, u32) {
        let head = format!("{}{with}", &self.text[..self.range.start]);
        let caret = utf16_len(&head);
        (format!("{head}{}", &self.text[self.range.end..]), caret)
    }
}

/// `text` after pasting `pasted` over the `start..end` selection, when it is a web or mail
/// address and something is selected: the link the selection is in gets the address, other
/// selected text becomes a link to it. Returns the new text and the caret after the link.
pub(crate) fn paste_link(text: &str, start: Option<u32>, end: Option<u32>, pasted: &str) -> Option<(String, u32)> {
    let address = pasted_link(pasted)?;
    let from = utf16_to_byte(text, start);
    let to = utf16_to_byte(text, end);
    if from >= to {
        return None;
    }
    let (range, caption) = match link_at(text, from, to) {
        Some(link) => (link.range, &text[link.caption]),
        None => (from..to, &text[from..to]),
    };
    let head = format!("{}{}", &text[..range.start], link_markup(caption, &address)?);
    let caret = utf16_len(&head);
    Some((format!("{head}{}", &text[range.end..]), caret))
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
        assert_eq!(utf16_to_byte("👍x", Some(1)), "👍".len());
    }

    #[test]
    fn deleting_keeps_markers() {
        // The first character of struck text: its markers stay.
        assert_eq!(delete_shown("a ~~bcd~~ e", 4, 5), Some(("a ~~cd~~ e".to_owned(), 4)));
        // The last one, also with the closing marker in the range.
        assert_eq!(delete_shown("a ~~bcd~~ e", 6, 7), Some(("a ~~bc~~ e".to_owned(), 6)));
        assert_eq!(delete_shown("a ~~bcd~~ e", 6, 9), Some(("a ~~bc~~ e".to_owned(), 6)));
        // Plain text next to a marker.
        assert_eq!(delete_shown("a ~~bcd~~ e", 1, 2), Some(("a~~bcd~~ e".to_owned(), 1)));
        assert_eq!(delete_shown("a ~~bcd~~ e", 9, 10), Some(("a ~~bcd~~e".to_owned(), 9)));
        // A selection from plain text into struck text.
        assert_eq!(delete_shown("a ~~bcd~~ e", 0, 5), Some(("~~cd~~ e".to_owned(), 0)));
        // All the struck text: the markers go too.
        assert_eq!(delete_shown("a ~~b~~ e", 4, 5), Some(("a  e".to_owned(), 2)));
        assert_eq!(delete_shown("a ~~bcd~~ e", 1, 10), Some(("ae".to_owned(), 1)));
        // A link keeps its address while its caption is edited.
        assert_eq!(
            delete_shown("[ab](https://x.ru)", 1, 2),
            Some(("[b](https://x.ru)".to_owned(), 1))
        );
        // Only markers in the range: nothing shown to delete.
        assert_eq!(delete_shown("a ~~b~~", 5, 7), Some(("a ~~b~~".to_owned(), 5)));
        assert_eq!(delete_shown("a ~~b~~", 7, 7), Some(("a ~~b~~".to_owned(), 7)));
        // Far from markers: the browser deletes.
        assert_eq!(delete_shown("ab ~~c~~", 0, 1), None);
        assert_eq!(delete_shown("abc", 1, 2), None);
        // UTF-16 offsets: "👍" is two units.
        assert_eq!(delete_shown("👍 **ab**", 5, 6), Some(("👍 **b**".to_owned(), 5)));
    }

    #[test]
    fn typing_at_the_edge_of_formatting_goes_outside_it() {
        // Right after a link's caption, in the middle of a line: after the link.
        assert_eq!(
            insert_shown("a [b](https://x.ru) c", 4, "x"),
            Some(("a [b](https://x.ru)x c".to_owned(), 20))
        );
        // Right before its caption: before the link.
        assert_eq!(
            insert_shown("a [b](https://x.ru) c", 3, "x"),
            Some(("a x[b](https://x.ru) c".to_owned(), 3))
        );
        // Inside a caption, at the edges of bold (it goes on until Ctrl+B ends it) or in plain text:
        // the browser types.
        assert_eq!(insert_shown("a [bc](https://x.ru) d", 4, "x"), None);
        assert_eq!(insert_shown("a **b** c", 5, "x"), None);
        assert_eq!(insert_shown("a **b** c", 1, "x"), None);
    }

    /// A bare address has no markers: next to it the browser types and deletes, and the caret
    /// steps as over text.
    #[test]
    fn a_bare_address_is_edited_as_text() {
        assert_eq!(delete_shown("a https://x.ru b", 15, 16), None);
        assert_eq!(delete_shown("a https://x.ru b", 1, 2), None);
        assert_eq!(insert_shown("a https://x.ru b", 14, "x"), None);
        assert_eq!(insert_shown("a https://x.ru b", 2, "x"), None);
        assert_eq!(past_markers("a https://x.ru", 2, false), 2);
        assert_eq!(past_markers("a https://x.ru", 14, true), 14);
    }

    #[test]
    fn caret_steps_over_markers() {
        assert_eq!(past_markers("a ~~b~~", 7, true), 5);
        assert_eq!(past_markers("a ~~b~~", 4, true), 2);
        assert_eq!(past_markers("a ~~b~~", 2, false), 4);
        assert_eq!(past_markers("~~a~~**b**", 5, true), 3);
        assert_eq!(past_markers("~~a~~**b**", 3, false), 7);
        assert_eq!(past_markers("a ~~b~~", 1, true), 1);
    }

    #[test]
    fn link_dialog_makes_and_edits_links() {
        // "docs" selected (UTF-16 offsets: "👍" is two units).
        let new = LinkTarget::new("👍 see docs now", Some(7), Some(11));
        assert_eq!((new.caption.as_str(), new.existing), ("docs", false));
        assert_eq!(
            new.save("docs", "e.org/a(1)"),
            Some(("👍 see [docs](https://e.org/a%281%29) now".to_owned(), 37))
        );
        assert_eq!(new.save("docs", ""), None);
        assert_eq!(new.save("a [b", "e.org"), None);
        assert_eq!(
            new.save(" ", "e.org"),
            Some(("👍 see https://e.org now".to_owned(), 20))
        );

        // The caret inside a link: the dialog edits that link.
        let text = "see [docs](https://e.org/a%28b%29) now";
        let old = LinkTarget::new(text, Some(6), Some(6));
        assert_eq!(
            (old.caption.as_str(), old.address.as_str(), old.existing),
            ("docs", "https://e.org/a(b)", true)
        );
        assert_eq!(
            old.save("the docs", "https://e.org/x"),
            Some(("see [the docs](https://e.org/x) now".to_owned(), 31))
        );
        assert_eq!(old.remove(), ("see docs now".to_owned(), 8));

        // Nothing selected: the link goes in at the caret.
        let empty = LinkTarget::new("ab", Some(1), Some(1));
        assert_eq!(empty.caption, "");
        assert_eq!(
            empty.save("x", "https://e.org"),
            Some(("a[x](https://e.org)b".to_owned(), 19))
        );
    }

    #[test]
    fn pasting_an_address_over_a_selection_links_it() {
        assert_eq!(
            paste_link("see docs now", Some(4), Some(8), "https://e.org/(x)\n"),
            Some(("see [docs](https://e.org/%28x%29) now".to_owned(), 33))
        );
        // Inside a link: its address changes.
        assert_eq!(
            paste_link("[docs](https://e.org) now", Some(1), Some(3), "mailto:olga@example.com"),
            Some(("[docs](mailto:olga@example.com) now".to_owned(), 31))
        );
        // Nothing selected, not an address, or a selection that cannot be a caption: an ordinary paste.
        assert_eq!(paste_link("see docs", Some(4), Some(4), "https://e.org"), None);
        assert_eq!(paste_link("see docs", Some(4), Some(8), "plain words"), None);
        assert_eq!(paste_link("a\nb", Some(0), Some(3), "https://e.org"), None);
    }
}
