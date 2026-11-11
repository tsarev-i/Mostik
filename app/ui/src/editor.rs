//! The message field: a plain-text `contenteditable` block that shows custom emoji as images, as
//! Slack's composer does, while its text keeps them as `:name:`. A textarea cannot show images.
//!
//! The field's text is what [`value`] reads back: text as is, an emoji image as its code, a line
//! break as `\n`. Offsets are UTF-16 units of that text, the same units a textarea's selection uses.

use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, Node};

/// Class of the `<br>` that lets a trailing line break show: it is not part of the text.
const TAIL: &str = "tail";

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

/// Fills the field with `text`, the codes for which `emoji` gives an image URL shown as images.
pub fn render(el: &HtmlElement, text: &str, emoji: &dyn Fn(&str) -> Option<String>) {
    el.set_inner_html("");
    let Some(doc) = el.owner_document() else { return };
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
            let _ = el.append_child(&node);
        }
    }
    if text.ends_with('\n')
        && let Ok(br) = doc.create_element("br")
    {
        let _ = br.set_attribute("class", TAIL);
        let _ = el.append_child(&br);
    }
}

/// Puts the caret at text offset `caret` of a field filled by [`render`].
pub fn set_caret(el: &HtmlElement, caret: u32) {
    let Some(doc) = el.owner_document() else { return };
    let Ok(range) = doc.create_range() else { return };
    let children = el.child_nodes();
    let mut acc = 0;
    let mut placed = false;
    for i in 0..children.length() {
        let Some(n) = children.item(i) else { continue };
        if n.node_type() == Node::TEXT_NODE {
            let len = utf16_len(&n.node_value().unwrap_or_default());
            if caret <= acc + len {
                placed = range.set_start(&n, caret - acc).is_ok();
                break;
            }
            acc += len;
        } else {
            let el_n: &Element = n.unchecked_ref();
            if el_n.class_list().contains(TAIL) {
                placed = range.set_start(el, i).is_ok();
                break;
            }
            if caret <= acc {
                placed = range.set_start(el, i).is_ok();
                break;
            }
            acc += match el_n.tag_name().as_str() {
                "IMG" => utf16_len(&el_n.get_attribute("data-code").unwrap_or_default()),
                "BR" => 1,
                _ => 0,
            };
        }
    }
    if !placed {
        let _ = range.select_node_contents(el);
        range.collapse_with_to_start(false);
    } else {
        range.collapse_with_to_start(true);
    }
    if let Some(sel) = web_sys::window().and_then(|w| w.get_selection().ok().flatten()) {
        let _ = sel.remove_all_ranges();
        let _ = sel.add_range(&range);
    }
}

/// Replaces the text of the field and puts the caret at `caret`.
pub fn set_text(el: &HtmlElement, text: &str, caret: u32, emoji: &dyn Fn(&str) -> Option<String>) {
    render(el, text, emoji);
    let _ = el.focus();
    set_caret(el, caret);
}

/// After typing or pasting: shows the newly complete `:name:` codes as images and flattens what
/// the browser added (blocks, stray line breaks), keeping the caret. An empty field is cleared, so
/// its placeholder shows. Returns the text.
pub fn tidy(el: &HtmlElement, emoji: &dyn Fn(&str) -> Option<String>) -> String {
    let text = value(el);
    if text.is_empty() {
        if el.child_nodes().length() > 0 {
            el.set_inner_html("");
        }
        return text;
    }
    let children = el.child_nodes();
    let mut flat = true;
    let mut codes = false;
    for i in 0..children.length() {
        let Some(n) = children.item(i) else { continue };
        match n.node_type() {
            Node::TEXT_NODE => {
                let t = n.node_value().unwrap_or_default();
                codes |= mostik_core::custom_emoji::split(&t, |c| emoji(c).is_some())
                    .iter()
                    .any(|p| matches!(p, mostik_core::custom_emoji::Piece::Emoji(_)));
            }
            _ => {
                let e: &Element = n.unchecked_ref();
                flat &= e.tag_name() == "IMG" || (e.tag_name() == "BR" && e.class_list().contains(TAIL));
            }
        }
    }
    // A trailing line break shows only with the tail `<br>` after it.
    let tail = children.item(children.length().saturating_sub(1)).is_some_and(|n| {
        n.node_type() == Node::ELEMENT_NODE && n.unchecked_ref::<Element>().class_list().contains(TAIL)
    });
    if flat && !codes && (text.ends_with('\n') == tail) {
        return text;
    }
    let caret = selection(el).0;
    render(el, &text, emoji);
    if let Some(caret) = caret {
        set_caret(el, caret);
    }
    text
}
