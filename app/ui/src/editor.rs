//! The message field: a plain-text `contenteditable` block that shows custom emoji as images and
//! markup as it will look (bold as bold, a link as its caption, a code block as a grey block,
//! markers hidden), as Slack's composer does, while its text keeps them as `:name:`, `**…**` and
//! `[caption](address)`. A textarea cannot show either.
//!
//! The field's text is what [`value`] reads back: text as is, an emoji image as its code, a line
//! break as `\n`. Offsets are UTF-16 units of that text, the same units a textarea's selection uses.

use mostik_core::markup::{EditSpan, Mark, edit_spans};
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, Node};

/// Class of the `<br>` that lets a trailing line break show: it is not part of the text.
const TAIL: &str = "tail";

/// Class of the span after formatted text with no plain text behind it: its zero-width space
/// (not part of the text) holds the caret outside the formatting, where the browser would
/// otherwise put it back inside.
const GAP: &str = "gap";
const ZWSP: char = '\u{200B}';

fn text_of(node: &Node, out: &mut String) {
    let children = node.child_nodes();
    for i in 0..children.length() {
        let Some(n) = children.item(i) else { continue };
        match n.node_type() {
            Node::TEXT_NODE => out.push_str(&n.node_value().unwrap_or_default()),
            Node::ELEMENT_NODE => {
                let el: &Element = n.unchecked_ref();
                match el.tag_name().as_str() {
                    "IMG" => out.push_str(&el.get_attribute("data-code").unwrap_or_default()),
                    "BR" if el.class_list().contains(TAIL) => {}
                    "BR" => out.push('\n'),
                    "SPAN" if el.class_list().contains(GAP) => {
                        out.extend(el.text_content().unwrap_or_default().chars().filter(|&c| c != ZWSP));
                    }
                    // A browser may put new lines in blocks.
                    "DIV" | "P" => {
                        if !out.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                        text_of(&n, out);
                    }
                    _ => text_of(&n, out),
                }
            }
            _ => {}
        }
    }
}

/// The text of the field.
pub fn value(el: &HtmlElement) -> String {
    let mut out = String::new();
    text_of(el, &mut out);
    // A browser keeps a `<br>` at the end of an emptied or edited block as a placeholder.
    let placeholder = el
        .last_child()
        .filter(|n| n.node_type() == Node::ELEMENT_NODE)
        .is_some_and(|n| {
            let e: &Element = n.unchecked_ref();
            e.tag_name() == "BR" && !e.class_list().contains(TAIL)
        });
    if placeholder {
        out.pop();
    }
    out
}

fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// Offset in the text of a DOM position inside the field.
fn offset_of(el: &HtmlElement, node: &Node, offset: u32) -> Option<u32> {
    if !el.contains(Some(node)) {
        return None;
    }
    let range = el.owner_document()?.create_range().ok()?;
    range.set_start(el, 0).ok()?;
    range.set_end(node, offset).ok()?;
    let mut out = String::new();
    let fragment: Node = range.clone_contents().ok()?.into();
    text_of(&fragment, &mut out);
    Some(utf16_len(&out))
}

/// The selection as text offsets; `None` when it is not in the field.
pub fn selection(el: &HtmlElement) -> (Option<u32>, Option<u32>) {
    let Some(range) = web_sys::window()
        .and_then(|w| w.get_selection().ok().flatten())
        .filter(|s| s.range_count() > 0)
        .and_then(|s| s.get_range_at(0).ok())
    else {
        return (None, None);
    };
    let at = |node: Option<Node>, offset: u32| node.and_then(|n| offset_of(el, &n, offset));
    (
        at(range.start_container().ok(), range.start_offset().unwrap_or(0)),
        at(range.end_container().ok(), range.end_offset().unwrap_or(0)),
    )
}

/// Class of the hidden markers: `**` of bold, the address of a link, the fences of a code block.
const MARKER: &str = "mk";

/// Appends `text` to `parent`, with the codes for which `emoji` gives an image URL as images.
fn append_text(doc: &web_sys::Document, parent: &Node, text: &str, emoji: &dyn Fn(&str) -> Option<String>) {
    for piece in mostik_core::custom_emoji::split(text, |n| emoji(n).is_some()) {
        let node: Option<Node> = match piece {
            mostik_core::custom_emoji::Piece::Text(t) => Some(doc.create_text_node(t).into()),
            mostik_core::custom_emoji::Piece::Emoji(name) => doc.create_element("img").ok().map(|img| {
                let code = mostik_core::custom_emoji::caption(name);
                let _ = img.set_attribute("class", "custom-emoji");
                let _ = img.set_attribute("src", &emoji(name).unwrap_or_default());
                let _ = img.set_attribute("alt", &code);
                let _ = img.set_attribute("title", &code);
                let _ = img.set_attribute("data-code", &code);
                let _ = img.set_attribute("draggable", "false");
                img.into()
            }),
        };
        if let Some(node) = node {
            let _ = parent.append_child(&node);
        }
    }
}

/// Class of the span that shows a kind of markup.
fn mark_class(kind: Mark) -> &'static str {
    match kind {
        Mark::Bold => "f-bold",
        Mark::Italic => "f-italic",
        Mark::Strike => "f-strike",
        Mark::Underline => "f-underline",
        Mark::Code => "f-code",
        Mark::CodeBlock => "f-code-block",
        Mark::Link => "f-link",
    }
}

/// Whether a marked text ends with a tail `<br>`: code whose last line is empty, which shows only
/// with something after it. A lone line break is the one empty line of a new block.
fn inner_tail(kind: Mark, inner: &str) -> bool {
    kind == Mark::CodeBlock && inner.len() > 1 && inner.ends_with('\n')
}

/// Whether the field ends with a tail `<br>`: after a line break, which shows only with something
/// after it, and after a code block, to give the caret a line under the block.
fn field_tail(text: &str, spans: &[EditSpan]) -> bool {
    text.ends_with('\n')
        || matches!(
            spans.last(),
            Some(EditSpan::Marked {
                kind: Mark::CodeBlock,
                ..
            })
        )
}

fn append_tail(doc: &web_sys::Document, parent: &Node) {
    if let Ok(br) = doc.create_element("br") {
        let _ = br.set_attribute("class", TAIL);
        let _ = parent.append_child(&br);
    }
}

/// Fills the field with `text`: markup shows as it will look (bold as bold, a link as its caption
/// with the address in its tooltip and so on) with its markers hidden, and the codes for which
/// `emoji` gives an image URL show as images.
pub fn render(el: &HtmlElement, text: &str, emoji: &dyn Fn(&str) -> Option<String>) {
    el.set_inner_html("");
    let Some(doc) = el.owner_document() else { return };
    let spans = edit_spans(text);
    let span_of = |class: &str, part: &str| {
        let span = doc.create_element("span").ok()?;
        let _ = span.set_attribute("class", class);
        append_text(&doc, &span, part, emoji);
        let _ = el.append_child(&span);
        Some(span)
    };
    for (i, span) in spans.iter().enumerate() {
        match *span {
            EditSpan::Plain(t) => append_text(&doc, el, t, emoji),
            EditSpan::Marked {
                kind,
                open,
                inner,
                close,
            } => {
                span_of(MARKER, open);
                if let Some(shown) = span_of(mark_class(kind), inner) {
                    if kind == Mark::Link
                        && let Some(address) = close.strip_prefix("](").and_then(|c| c.strip_suffix(')'))
                    {
                        let _ = shown.set_attribute("title", address);
                    }
                    if inner_tail(kind, inner) {
                        append_tail(&doc, &shown);
                    }
                }
                span_of(MARKER, close);
                // A code block is a block: the caret has its own lines around it.
                let plain_next = matches!(spans.get(i + 1), Some(EditSpan::Plain(t)) if !t.starts_with('\n'));
                if kind != Mark::CodeBlock
                    && !plain_next
                    && let Ok(gap) = doc.create_element("span")
                {
                    let _ = gap.set_attribute("class", GAP);
                    gap.set_text_content(Some(&ZWSP.to_string()));
                    let _ = el.append_child(&gap);
                }
            }
        }
    }
    if field_tail(text, &spans) {
        append_tail(&doc, el);
    }
}

/// A plain run or a span of the field: its class (empty for plain text), its text and whether it
/// ends with a tail `<br>`.
type Piece = (String, String, bool);

/// What the field is made of: plain runs and spans, in order; `None` when the browser added
/// something of its own (blocks, line breaks), so it needs rendering again.
fn shape(el: &HtmlElement) -> Option<Vec<Piece>> {
    let mut out: Vec<Piece> = Vec::new();
    let mut plain = String::new();
    let children = el.child_nodes();
    for i in 0..children.length() {
        let Some(n) = children.item(i) else { continue };
        match n.node_type() {
            Node::TEXT_NODE => plain.push_str(&n.node_value().unwrap_or_default()),
            Node::ELEMENT_NODE => {
                let e: &Element = n.unchecked_ref();
                match e.tag_name().as_str() {
                    "IMG" => plain.push_str(&e.get_attribute("data-code").unwrap_or_default()),
                    "BR" if e.class_list().contains(TAIL) => {}
                    "SPAN"
                        if e.class_list().contains(GAP) && e.text_content().unwrap_or_default() == ZWSP.to_string() => {
                    }
                    "SPAN" => {
                        if !plain.is_empty() {
                            out.push((String::new(), std::mem::take(&mut plain), false));
                        }
                        let mut t = String::new();
                        text_of(&n, &mut t);
                        let tail = e
                            .last_element_child()
                            .is_some_and(|b| b.tag_name() == "BR" && b.class_list().contains(TAIL));
                        out.push((e.class_name(), t, tail));
                    }
                    _ => return None,
                }
            }
            _ => {}
        }
    }
    if !plain.is_empty() {
        out.push((String::new(), plain, false));
    }
    Some(out)
}

/// The shape [`render`] gives `spans`.
fn shape_of(spans: &[EditSpan]) -> Vec<Piece> {
    let mut out = Vec::new();
    for span in spans {
        match *span {
            EditSpan::Plain(t) => out.push((String::new(), t.to_owned(), false)),
            EditSpan::Marked {
                kind,
                open,
                inner,
                close,
            } => {
                out.push((MARKER.to_owned(), open.to_owned(), false));
                out.push((mark_class(kind).to_owned(), inner.to_owned(), inner_tail(kind, inner)));
                out.push((MARKER.to_owned(), close.to_owned(), false));
            }
        }
    }
    out
}

/// Text nodes, images and line breaks of the field in document order, with their text length,
/// whether they are hidden markers and how many units before the text the node has (the
/// zero-width space of a gap).
fn leaves(node: &Node, hidden: bool, out: &mut Vec<(Node, u32, bool, u32)>) {
    let children = node.child_nodes();
    for i in 0..children.length() {
        let Some(n) = children.item(i) else { continue };
        match n.node_type() {
            Node::TEXT_NODE => {
                let t = n.node_value().unwrap_or_default();
                let in_gap = node.unchecked_ref::<Element>().class_list().contains(GAP);
                let skew = if in_gap && t.starts_with(ZWSP) { 1 } else { 0 };
                let len = utf16_len(&t)
                    - if in_gap {
                        t.chars().filter(|&c| c == ZWSP).count() as u32
                    } else {
                        0
                    };
                out.push((n, len, hidden, skew));
            }
            Node::ELEMENT_NODE => {
                let e: &Element = n.unchecked_ref();
                match e.tag_name().as_str() {
                    "IMG" => {
                        let len = utf16_len(&e.get_attribute("data-code").unwrap_or_default());
                        out.push((n, len, hidden, 0));
                    }
                    "BR" if e.class_list().contains(TAIL) => out.push((n, 0, hidden, 0)),
                    "BR" => out.push((n, 1, hidden, 0)),
                    _ => {
                        leaves(&n, hidden || e.class_list().contains(MARKER), out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Whether `node` is the tail `<br>` of the field itself, which takes the offsets past the end; the
/// tail of a code block is in its span.
fn is_field_tail(el: &HtmlElement, node: &Node) -> bool {
    node.unchecked_ref::<Element>().class_list().contains(TAIL)
        && node.parent_node().is_some_and(|p| el.is_same_node(Some(&p)))
}

/// The DOM position of text offset `at` in a field filled by [`render`]: never inside a hidden
/// marker; `None` means the end of the field.
fn position(el: &HtmlElement, at: u32) -> Option<(Node, u32)> {
    let mut all = Vec::new();
    leaves(el, false, &mut all);
    let mut acc = 0;
    for (n, len, hidden, skew) in all {
        if hidden {
            acc += len;
            continue;
        }
        if n.node_type() == Node::TEXT_NODE {
            if at <= acc + len {
                return Some((n, at.saturating_sub(acc) + skew));
            }
        } else if at <= acc || is_field_tail(el, &n) {
            let parent = n.parent_node()?;
            let children = parent.child_nodes();
            let index = (0..children.length()).find(|&i| children.item(i).as_ref() == Some(&n))?;
            return Some((parent, index));
        }
        acc += len;
    }
    None
}

/// Selects text offsets `start..end` of a field filled by [`render`]; equal offsets put the caret.
pub fn set_selection(el: &HtmlElement, start: u32, end: u32) {
    let Some(doc) = el.owner_document() else { return };
    let Ok(range) = doc.create_range() else { return };
    let set = |at: u32, start: bool| match position(el, at) {
        Some((n, o)) if start => range.set_start(&n, o).is_ok(),
        Some((n, o)) => range.set_end(&n, o).is_ok(),
        None => {
            let len = el.child_nodes().length();
            if start {
                range.set_start(el, len).is_ok()
            } else {
                range.set_end(el, len).is_ok()
            }
        }
    };
    set(start, true);
    if end > start {
        set(end, false);
    } else {
        range.collapse_with_to_start(true);
    }
    if let Some(sel) = web_sys::window().and_then(|w| w.get_selection().ok().flatten()) {
        let _ = sel.remove_all_ranges();
        let _ = sel.add_range(&range);
    }
}

/// Puts the caret at text offset `caret` of a field filled by [`render`].
pub fn set_caret(el: &HtmlElement, caret: u32) {
    set_selection(el, caret, caret);
}

/// Replaces the text of the field and puts the caret at `caret`.
pub fn set_text(el: &HtmlElement, text: &str, caret: u32, emoji: &dyn Fn(&str) -> Option<String>) {
    render(el, text, emoji);
    let _ = el.focus();
    set_caret(el, caret);
}

/// After typing or pasting: shows newly complete markup and `:name:` codes as they will look and
/// flattens what the browser added (blocks, stray line breaks), keeping the caret. An empty field
/// is cleared, so its placeholder shows. Returns the text.
pub fn tidy(el: &HtmlElement, emoji: &dyn Fn(&str) -> Option<String>) -> String {
    let text = value(el);
    if text.is_empty() {
        if el.child_nodes().length() > 0 {
            el.set_inner_html("");
        }
        return text;
    }
    let mut all = Vec::new();
    leaves(el, false, &mut all);
    let codes = all.iter().any(|(n, ..)| {
        n.node_type() == Node::TEXT_NODE
            && mostik_core::custom_emoji::split(&n.node_value().unwrap_or_default(), |c| emoji(c).is_some())
                .iter()
                .any(|p| matches!(p, mostik_core::custom_emoji::Piece::Emoji(_)))
    });
    let children = el.child_nodes();
    let tail = children.item(children.length().saturating_sub(1)).is_some_and(|n| {
        n.node_type() == Node::ELEMENT_NODE && n.unchecked_ref::<Element>().class_list().contains(TAIL)
    });
    let spans = edit_spans(&text);
    if !codes && field_tail(&text, &spans) == tail && shape(el).is_some_and(|s| s == shape_of(&spans)) {
        return text;
    }
    let caret = selection(el).0;
    render(el, &text, emoji);
    if let Some(caret) = caret {
        set_caret(el, caret);
    }
    text
}

/// The marked spans of the field's text as byte offsets: where each one starts, where its text
/// starts and ends, and where it ends.
fn marked_spans(text: &str) -> Vec<[usize; 4]> {
    let mut out = Vec::new();
    let mut at = 0;
    for span in edit_spans(text) {
        match span {
            EditSpan::Plain(t) => at += t.len(),
            EditSpan::Marked { open, inner, close, .. } => {
                let inner_at = at + open.len();
                let close_at = inner_at + inner.len();
                out.push([at, inner_at, close_at, close_at + close.len()]);
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
fn delete_shown(text: &str, start: u32, end: u32) -> Option<(String, u32)> {
    let from = crate::app::utf16_to_byte(text, Some(start));
    let to = crate::app::utf16_to_byte(text, Some(end)).max(from);
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
fn past_markers(text: &str, at: u32, back: bool) -> u32 {
    let spans = marked_spans(text);
    let mut at = crate::app::utf16_to_byte(text, Some(at));
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

/// Before the browser deletes text: a deletion in or next to the hidden markers is made here,
/// since the browser would take the markers along and the markup would fall apart (a fragment
/// with its first character deleted would show its closing `~~`). The field changes as if typed in.
pub fn before_input(el: &HtmlElement, ev: &web_sys::InputEvent, emoji: &dyn Fn(&str) -> Option<String>) {
    let kind = ev.input_type();
    let (back, unit) = match kind.as_str() {
        "deleteContentBackward" => (true, "character"),
        "deleteWordBackward" => (true, "word"),
        "deleteSoftLineBackward" | "deleteHardLineBackward" => (true, "lineboundary"),
        "deleteContentForward" | "deleteContent" | "deleteByCut" => (false, "character"),
        "deleteWordForward" => (false, "word"),
        "deleteSoftLineForward" | "deleteHardLineForward" | "deleteEntireSoftLine" => (false, "lineboundary"),
        _ => return,
    };
    if ev.is_composing() {
        return;
    }
    let Ok(range) = ev.get_target_ranges().get(0).dyn_into::<web_sys::StaticRange>() else {
        return;
    };
    let (Some(start), Some(end)) = (
        offset_of(el, &range.start_container(), range.start_offset()),
        offset_of(el, &range.end_container(), range.end_offset()),
    ) else {
        return;
    };
    let text = value(el);
    let Some((mut shown, mut caret)) = delete_shown(&text, start, end) else {
        return;
    };
    ev.prevent_default();
    // Nothing shown in the range, only markers or the zero-width space after them: the caret is
    // past formatted text, and the character before it (or after) is in that text.
    if shown == text {
        let to = past_markers(&text, if back { start } else { end }, back);
        let byte = |at: u32| crate::app::utf16_to_byte(&text, Some(at));
        // Under a code block the caret is on a line of its own: going back takes it into the block.
        if text[byte(start.min(to))..byte(end.max(to))].contains("```") {
            if back {
                set_caret(el, to);
            }
            return;
        }
        let Some(sel) = web_sys::window().and_then(|w| w.get_selection().ok().flatten()) else {
            return;
        };
        let before = selection(el);
        set_caret(el, to);
        let _ = sel.modify("extend", if back { "backward" } else { "forward" }, unit);
        let deleted = match selection(el) {
            (Some(start), Some(end)) => delete_shown(&text, start, end),
            _ => None,
        };
        match deleted {
            Some(deleted) if deleted.0 != text => (shown, caret) = deleted,
            _ => {
                if let (Some(start), Some(end)) = before {
                    set_selection(el, start, end);
                }
                return;
            }
        }
    }
    render(el, &shown, emoji);
    set_caret(el, caret);
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    if let Ok(input) = web_sys::Event::new_with_event_init_dict("input", &init) {
        let _ = el.dispatch_event(&input);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn caret_steps_over_markers() {
        assert_eq!(past_markers("a ~~b~~", 7, true), 5);
        assert_eq!(past_markers("a ~~b~~", 4, true), 2);
        assert_eq!(past_markers("a ~~b~~", 2, false), 4);
        assert_eq!(past_markers("~~a~~**b**", 5, true), 3);
        assert_eq!(past_markers("~~a~~**b**", 3, false), 7);
        assert_eq!(past_markers("a ~~b~~", 1, true), 1);
    }
}
