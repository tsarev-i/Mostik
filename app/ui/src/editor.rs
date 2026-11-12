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
