//! What the three message fields share (the chat composer, the thread reply and the in-place
//! editor of a message), as in Slack: the formatting toolbar and its hotkeys, the "Add link"
//! dialog and a link pasted over selected text.

use std::ops::Range;

use leptos::ev;
use leptos::html;
use leptos::portal::Portal;
use leptos::prelude::*;
use mostik_core::markup::{link_address, link_at, link_markup, pasted_link, typed_address};

use crate::app::{
    Ctx, insert_into, place_caret, scoped_window_listener, selection, utf16_len, utf16_to_byte, wrap_selection,
};
use crate::editor;
use crate::i18n::tr;
use crate::icons::{I, Icon};

/// A message field as its toolbar, hotkeys and dialogs see it.
#[derive(Clone, Copy)]
pub struct Field {
    area: NodeRef<html::Div>,
    /// Takes the text of the field after a change made here.
    changed: Callback<String>,
    /// What the open "Add link" dialog edits.
    link: RwSignal<Option<LinkTarget>>,
}

impl Field {
    pub fn new(area: NodeRef<html::Div>, changed: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            area,
            changed: Callback::new(changed),
            link: RwSignal::new(None),
        }
    }

    /// Wraps the selection in markup markers or takes them off, as the toolbar buttons do.
    pub fn wrap(self, before: &str, after: &str) {
        if let Some(el) = self.area.get_untracked() {
            self.changed.run(wrap_selection(&el, before, after));
        }
    }

    /// Puts `text` in place of the `start..end` selection kept from before a picker took the focus.
    fn insert_at(self, text: &str, (start, end): (Option<u32>, Option<u32>)) {
        if let Some(el) = self.area.get_untracked() {
            let (value, caret) = insert_into(&editor::value(&el), start, end, text, 0);
            place_caret(&el, &value, caret);
            self.changed.run(value);
        }
    }

    /// Opens the "Add link" dialog for the link the caret is in, or for the selected text.
    pub fn open_link(self) {
        if let Some(el) = self.area.get_untracked() {
            let (start, end) = selection(&el);
            self.link.set(Some(LinkTarget::new(&editor::value(&el), start, end)));
        }
    }

    /// Closes the link dialog with the text it made, or with nothing to put the selection back.
    fn close_link(self, made: Option<(String, u32)>) {
        let Some(target) = self.link.get_untracked() else {
            return;
        };
        self.link.set(None);
        let Some(el) = self.area.get_untracked() else { return };
        match made {
            // The field cannot change under the dialog; if it did, the link is not put over it.
            Some((text, caret)) if editor::value(&el) == target.text => {
                place_caret(&el, &text, caret);
                self.changed.run(text);
            }
            _ => {
                let _ = el.focus();
                editor::set_selection(&el, target.selection.0, target.selection.1);
            }
        }
    }

    /// Formatting hotkeys, as in Slack: Ctrl+B bold, Ctrl+I italic, Ctrl+Shift+X strikethrough,
    /// Ctrl+Shift+U a link. Keys are taken by their place, so they work in any keyboard layout.
    /// Returns whether the key was one of them.
    pub fn hotkey(self, e: &ev::KeyboardEvent) -> bool {
        if !(e.ctrl_key() || e.meta_key()) || e.alt_key() {
            return false;
        }
        match (e.code().as_str(), e.shift_key()) {
            ("KeyB", false) => self.wrap("**", "**"),
            ("KeyI", false) => self.wrap("__", "__"),
            ("KeyX", true) => self.wrap("~~", "~~"),
            ("KeyU", true) => self.open_link(),
            _ => return false,
        }
        e.prevent_default();
        true
    }

    /// Ctrl+V of a web or mail address over selected text, as in Slack: the text becomes a link
    /// to it, or the link the selection is in gets the new address. Returns whether it did.
    pub fn paste_link(self, e: &ev::ClipboardEvent) -> bool {
        let Some(el) = self.area.get_untracked() else {
            return false;
        };
        let Some(pasted) = e.clipboard_data().and_then(|d| d.get_data("text/plain").ok()) else {
            return false;
        };
        let (start, end) = selection(&el);
        let Some((text, caret)) = paste_link(&editor::value(&el), start, end, &pasted) else {
            return false;
        };
        e.prevent_default();
        place_caret(&el, &text, caret);
        self.changed.run(text);
        true
    }
}

/// What the link dialog edits: the link the selection is in, or the selected text to make a link of.
#[derive(Clone, Debug, PartialEq)]
struct LinkTarget {
    /// The text of the field when the dialog opened, and the selection in it.
    text: String,
    selection: (u32, u32),
    /// Bytes of `text` the link takes the place of.
    range: Range<usize>,
    caption: String,
    address: String,
    /// An existing link, which the dialog can also remove.
    existing: bool,
}

impl LinkTarget {
    fn new(text: &str, start: Option<u32>, end: Option<u32>) -> Self {
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
    fn save(&self, caption: &str, address: &str) -> Option<(String, u32)> {
        let address = link_address(address)?;
        if caption.trim().is_empty() {
            return Some(self.replace(&address));
        }
        Some(self.replace(&link_markup(caption, &address)?))
    }

    /// The text with the link taken off: its caption stays as plain text.
    fn remove(&self) -> (String, u32) {
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
fn paste_link(text: &str, start: Option<u32>, end: Option<u32>, pasted: &str) -> Option<(String, u32)> {
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

/// Where the emoji picker of a field opens, and the field's selection it inserts into: the
/// picker's search takes the focus.
type PickerAt = ((f64, f64), (Option<u32>, Option<u32>));

/// Formatting buttons of a message field: each wraps the selection in Telemost markup markers or
/// takes them off, "Link" opens the "Add link" dialog. A press keeps the focus and selection in the
/// field. `emoji` adds an emoji button, for a field without one of its own.
#[component]
pub fn FormatBar(field: Field, #[prop(optional)] emoji: bool) -> impl IntoView {
    let fmt = move |before: &'static str, after: &'static str| move |_| field.wrap(before, after);
    let hint = |name: &str, keys: &str| format!("{name} ({keys})");
    let picker: RwSignal<Option<PickerAt>> = RwSignal::new(None);
    let open_picker = move |e: ev::MouseEvent| {
        if let Some(el) = field.area.get_untracked() {
            picker.set(Some((crate::emoji::anchor_of(&e), selection(&el))));
        }
    };
    view! {
        <div class="composer-toolbar" on:mousedown=|e: ev::MouseEvent| e.prevent_default()>
            <button class="fmt" title=hint(tr().fmt_bold, "Ctrl+B") on:click=fmt("**", "**")><Icon icon=I::Bold size=16/></button>
            <button class="fmt" title=hint(tr().fmt_italic, "Ctrl+I") on:click=fmt("__", "__")><Icon icon=I::Italic size=16/></button>
            <button class="fmt" title=tr().fmt_underline on:click=fmt("++", "++")><Icon icon=I::Underline size=16/></button>
            <button class="fmt" title=hint(tr().fmt_strike, "Ctrl+Shift+X") on:click=fmt("~~", "~~")><Icon icon=I::Strike size=16/></button>
            <span class="sep"></span>
            <button class="fmt" title=hint(tr().fmt_link, "Ctrl+Shift+U") on:click=move |_| field.open_link()><Icon icon=I::Link size=16/></button>
            <span class="sep"></span>
            <button class="fmt" title=tr().fmt_code on:click=fmt("`", "`")><Icon icon=I::Code size=16/></button>
            <button class="fmt" title=tr().fmt_code_block on:click=fmt("```\n", "\n```")><Icon icon=I::CodeBlock size=16/></button>
            {emoji.then(|| view! {
                <span class="sep"></span>
                <button class="fmt" title=tr().emoji_btn on:click=open_picker><Icon icon=I::Smile size=16/></button>
            })}
        </div>
        {move || field.link.get().map(|target| view! { <LinkDialog field target/> })}
        {move || picker.get().map(|(at, sel)| {
            let c = expect_context::<Ctx>();
            view! {
                <crate::emoji::EmojiPicker source=crate::emoji::Source::Text anchor=(at.0 + 324.0, at.1)
                    custom=crate::custom::for_chat(c, &c.current.get_untracked()) can_add=c.live
                    on_close=Callback::new(move |_| {
                        picker.set(None);
                        if let Some(el) = field.area.get_untracked() {
                            let _ = el.focus();
                        }
                    })
                    on_pick=Callback::new(move |glyph: String| {
                        picker.set(None);
                        field.insert_at(&glyph, sel);
                    })/>
            }
        })}
    }
}

/// "Add link", as in Slack: the caption (the selected text) and the address; for the link the
/// caret is in, "Edit link" that can also remove it.
#[component]
fn LinkDialog(field: Field, target: LinkTarget) -> impl IntoView {
    let caption = RwSignal::new(target.caption.clone());
    let address = RwSignal::new(target.address.clone());
    let existing = target.existing;
    let target = StoredValue::new(target);
    let made = move || target.with_value(|t| t.save(&caption.get(), &address.get()));
    let save = move || {
        if let Some(made) = target.with_value(|t| t.save(&caption.get_untracked(), &address.get_untracked())) {
            field.close_link(Some(made));
        }
    };
    let enter = move |e: ev::KeyboardEvent| {
        if e.key() == "Enter" {
            e.prevent_default();
            save();
        }
    };
    // Esc closes this dialog only (the App handler leaves nested dialogs alone).
    scoped_window_listener(ev::keydown, move |e| {
        if e.key() == "Escape" {
            field.close_link(None);
        }
    });
    let caption_ref = NodeRef::<html::Input>::new();
    let address_ref = NodeRef::<html::Input>::new();
    // The address, when the caption is there already, as in Slack.
    Effect::new(move |_| {
        let input = if caption.get_untracked().is_empty() {
            caption_ref.get()
        } else {
            address_ref.get()
        };
        if let Some(input) = input {
            let _ = input.focus();
        }
    });
    view! {
        <Portal>
            <div class="modal-backdrop nested" on:click=move |_| field.close_link(None)>
                <div class="modal link-dialog" on:click=|e| e.stop_propagation()>
                    <header>
                        <h2>{if existing { tr().link_edit } else { tr().link_add }}</h2>
                        <button class="icon-btn" title=tr().close on:click=move |_| field.close_link(None)><Icon icon=I::X/></button>
                    </header>
                    <label class="field-label">{tr().link_text}</label>
                    <input class="text-input" node_ref=caption_ref prop:value=move || caption.get()
                        on:input=move |e| caption.set(event_target_value(&e)) on:keydown=enter/>
                    <label class="field-label">{tr().link_address}</label>
                    <input class="text-input" node_ref=address_ref placeholder="https://" prop:value=move || address.get()
                        on:input=move |e| address.set(event_target_value(&e)) on:keydown=enter/>
                    <div class="modal-actions">
                        {existing.then(|| view! {
                            <button class="btn" on:click=move |_| field.close_link(Some(target.with_value(LinkTarget::remove)))>
                                {tr().link_remove}
                            </button>
                            <span class="spacer"></span>
                        })}
                        <button class="btn" on:click=move |_| field.close_link(None)>{tr().cancel}</button>
                        <button class="btn primary" disabled=move || made().is_none() on:click=move |_| save()>{tr().save}</button>
                    </div>
                </div>
            </div>
        </Portal>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
