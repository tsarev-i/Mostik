//! What the three message fields share (the chat composer, the thread reply and the in-place
//! editor of a message), as in Slack: the formatting toolbar and its hotkeys, the "Add link"
//! dialog, a link pasted over selected text, and files pasted or dropped into a message.

use leptos::ev;
use leptos::html;
use leptos::portal::Portal;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::{ChatId, Timestamp};
use wasm_bindgen::JsCast;

use crate::app::{Ctx, place_caret, scoped_window_listener, selection, wrap_selection};
use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::keys::{self, Action};
use crate::modal::Modal;
use crate::text_edit::{LinkTarget, insert_into, paste_link};
use crate::{bridge, editor};

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
    /// Ctrl+Shift+U or Ctrl+Shift+K a link; Cmd in place of Ctrl on macOS, where Ctrl+B and Ctrl+I
    /// stay the field's. Or the keys the user gave them (see [`keys`]). Returns whether the key was
    /// one of them.
    pub fn hotkey(self, e: &ev::KeyboardEvent) -> bool {
        if keys::pressed(e, Action::Bold) {
            self.wrap("**", "**");
        } else if keys::pressed(e, Action::Italic) {
            self.wrap("__", "__");
        } else if keys::pressed(e, Action::Strike) {
            self.wrap("~~", "~~");
        } else if keys::pressed(e, Action::Link) {
            self.open_link();
        } else {
            return false;
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

/// The `contenteditable` of a message field, one for the composer of a chat or thread and for the
/// in-place editor of a message: it shows its text formatted with the markers hidden (see
/// [`editor`]) and makes the changes next to the markers itself. A key goes to the input method
/// first, then to the formatting hotkeys, then to `on_key` (suggestions, Enter, Esc). A paste goes
/// to `on_paste` first (files), then makes a link of selected text, else the browser pastes.
#[component]
pub fn FieldArea(
    field: Field,
    /// The text after the user typed, deleted or pasted.
    #[prop(into)]
    on_input: Callback<String>,
    #[prop(into)] on_key: Callback<ev::KeyboardEvent>,
    /// Takes a paste before the field does; returns whether it took it.
    #[prop(optional, into)]
    on_paste: Option<Callback<ev::ClipboardEvent, bool>>,
    #[prop(optional, into)] on_blur: Option<Callback<()>>,
    #[prop(optional, into)] placeholder: MaybeProp<String>,
) -> impl IntoView {
    let c = expect_context::<Ctx>();
    let area = field.area;
    view! {
        <div class="editor" node_ref=area contenteditable="plaintext-only" role="textbox" aria-multiline="true"
            aria-placeholder=move || placeholder.get()
            on:input=move |_| {
                if let Some(el) = area.get_untracked() {
                    on_input.run(editor::tidy(&el, &crate::custom::lookup(c)));
                }
            }
            on:beforeinput=move |e: ev::InputEvent| {
                if let Some(el) = area.get_untracked() {
                    editor::before_input(&el, &e, &crate::custom::lookup(c));
                }
            }
            // The window's shortcuts are not the field's: given keys like Ctrl+↑, zoom would also
            // edit the last message.
            on:keydown=move |e: ev::KeyboardEvent| {
                if !e.is_composing() && !field.hotkey(&e) && !keys::window_shortcut(&e) {
                    on_key.run(e);
                }
            }
            on:paste=move |e: ev::ClipboardEvent| {
                if !on_paste.is_some_and(|take| take.run(e.clone())) {
                    field.paste_link(&e);
                }
            }
            // A field removed with the focus in it (the chat switched to "Files" or "Pinned") gets
            // `blur` after its component, `on_blur` with it, is disposed (#61).
            on:blur=move |_| {
                if let Some(blur) = on_blur {
                    blur.try_run(());
                }
            }
        ></div>
    }
}

/// Where the emoji picker of a field opens, and the field's selection it inserts into: the
/// picker's search takes the focus.
type PickerAt = ((f64, f64), (Option<u32>, Option<u32>));

/// An emoji button for a message field: opens the picker and inserts the emoji where the caret or
/// selection was when it opened, then gives the focus back to the field.
#[component]
pub fn EmojiButton(field: Field, class: &'static str, size: u32) -> impl IntoView {
    let picker: RwSignal<Option<PickerAt>> = RwSignal::new(None);
    let open_picker = move |e: ev::MouseEvent| {
        if let Some(el) = field.area.get_untracked() {
            picker.set(Some((crate::emoji::anchor_of(&e), selection(&el))));
        }
    };
    view! {
        // The press keeps the focus and selection in the field.
        <button class=class title=tr().emoji_btn on:mousedown=|e: ev::MouseEvent| e.prevent_default() on:click=open_picker>
            <Icon icon=I::Smile size=size/>
        </button>
        {move || picker.get().map(|(at, sel)| {
            let c = expect_context::<Ctx>();
            // Read once: the picker would be built again, losing what is typed in its search, at
            // every change of the snapshot or the emoji sets.
            let custom = untrack(|| crate::custom::for_chat(c, &c.current.get_untracked()));
            view! {
                <crate::emoji::EmojiPicker source=crate::emoji::Source::Text anchor=(at.0 + 324.0, at.1)
                    custom=custom can_add=c.live
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

/// Formatting buttons of a message field: each wraps the selection in Telemost markup markers or
/// takes them off, "Link" opens the "Add link" dialog. A press keeps the focus and selection in the
/// field. `emoji` adds an emoji button, for a field without one of its own.
#[component]
pub fn FormatBar(field: Field, #[prop(optional)] emoji: bool) -> impl IntoView {
    let fmt = move |before: &'static str, after: &'static str| move |_| field.wrap(before, after);
    let c = expect_context::<Ctx>();
    let hint = move |name: &'static str, action: Action| {
        move || format!("{name} ({})", c.settings.with(|s| keys::shown(&s.shortcuts, action)))
    };
    view! {
        <div class="composer-toolbar" on:mousedown=|e: ev::MouseEvent| e.prevent_default()>
            <button class="fmt" title=hint(tr().fmt_bold, Action::Bold) on:click=fmt("**", "**")><Icon icon=I::Bold size=16/></button>
            <button class="fmt" title=hint(tr().fmt_italic, Action::Italic) on:click=fmt("__", "__")><Icon icon=I::Italic size=16/></button>
            <button class="fmt" title=tr().fmt_underline on:click=fmt("++", "++")><Icon icon=I::Underline size=16/></button>
            <button class="fmt" title=hint(tr().fmt_strike, Action::Strike) on:click=fmt("~~", "~~")><Icon icon=I::Strike size=16/></button>
            <span class="sep"></span>
            <button class="fmt" title=hint(tr().fmt_link, Action::Link) on:click=move |_| field.open_link()><Icon icon=I::Link size=16/></button>
            <span class="sep"></span>
            <button class="fmt" title=tr().fmt_code on:click=fmt("`", "`")><Icon icon=I::Code size=16/></button>
            <button class="fmt" title=tr().fmt_code_block on:click=fmt("```\n", "\n```")><Icon icon=I::CodeBlock size=16/></button>
            {emoji.then(|| view! {
                <span class="sep"></span>
                <EmojiButton field class="fmt" size=16/>
            })}
        </div>
        {move || field.link.get().map(|target| view! { <LinkDialog field target/> })}
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
            <Modal nested=true on_close=Callback::new(move |_| field.close_link(None))>
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
            </Modal>
        </Portal>
    }
}

/// Files pasted or dropped into a message are attached to it, as with "+": the app keeps their
/// contents until the message goes. `thread` is the parent of the thread whose field takes them,
/// `None` for the chat's own field. Returns whether there were any.
fn attach(c: Ctx, data: Option<web_sys::DataTransfer>, thread: Option<Timestamp>) -> bool {
    let files: Vec<(String, web_sys::Blob)> = data
        .and_then(|d| d.files())
        .map(|list| {
            (0..list.length())
                .filter_map(|i| list.get(i))
                .map(|f| (file_name(&f.name(), &f.type_()), f.into()))
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        return false;
    }
    if !c.live {
        c.error.set(Some(tr().only_in_app_files.into()));
        return true;
    }
    let chat = c.current.get_untracked();
    spawn_local(keep_all(c, chat, thread, files));
    true
}

/// Hands the `files` (name and contents) to the app and attaches them to the message in `chat`
/// (in the thread of `thread`). A large file takes a while to read: it goes to the chat or thread
/// it was pasted in, even if the user has moved on.
async fn keep_all(c: Ctx, chat: ChatId, thread: Option<Timestamp>, files: Vec<(String, web_sys::Blob)>) {
    for (name, blob) in files {
        let kept = match wasm_bindgen_futures::JsFuture::from(blob.array_buffer()).await {
            Ok(data) => bridge::keep_file(&name, data.unchecked_ref()).await,
            Err(e) => Err(format!("{name}: {e:?}")),
        };
        match kept {
            Ok(file) => crate::ops::attach_files(c, chat.clone(), thread, vec![file]),
            Err(e) => return c.error.set(Some(e)),
        }
    }
}

/// Files pasted into a message are attached to it, unless they come with formatted text: a table
/// copied from a spreadsheet or a document brings a picture of itself along, and then the text is
/// what is meant. `thread` as in [`attach`]. Returns whether the paste was taken.
pub fn attach_pasted(c: Ctx, e: &ev::ClipboardEvent, thread: Option<Timestamp>) -> bool {
    let Some(data) = e.clipboard_data() else { return false };
    let has = |kind: &str| data.get_data(kind).is_ok_and(|t| !t.trim().is_empty());
    if has("text/plain") && has("text/html") {
        return false;
    }
    if attach(c, Some(data.clone()), thread) {
        e.prevent_default();
        return true;
    }
    // Spaces and line breaks alone are pasted too.
    let any = |kind: &str| data.get_data(kind).is_ok_and(|t| !t.is_empty());
    if any("text/plain") || any("text/html") || any("text/uri-list") || !c.live {
        return false;
    }
    // Nothing the field could paste: the WebKitGTK the AppImage carries gives a picture alone in
    // the clipboard (a screenshot) to the paste event neither as a file nor as a type, but the
    // Clipboard API, asked in the paste, reads it.
    e.prevent_default();
    let read = window().navigator().clipboard().read();
    let chat = c.current.get_untracked();
    spawn_local(async move {
        let Ok(items) = wasm_bindgen_futures::JsFuture::from(read).await else {
            return;
        };
        let mut files = Vec::new();
        for item in js_sys::Array::from(&items).iter() {
            let item: web_sys::ClipboardItem = item.unchecked_into();
            let types: Vec<String> = item.types().iter().filter_map(|t| t.as_string()).collect();
            let Some(kind) = picture_type(&types) else { continue };
            if let Ok(blob) = wasm_bindgen_futures::JsFuture::from(item.get_type(kind)).await {
                files.push((file_name("", kind), blob.unchecked_into()));
            }
        }
        keep_all(c, chat, thread, files).await;
    });
    true
}

/// The type a clipboard item offering `types` is taken as a picture in: PNG if it is one of them,
/// as the copy loses nothing, else the first picture type; `None` when it is no picture.
fn picture_type(types: &[String]) -> Option<&str> {
    let picture = |t: &&String| t.starts_with("image/");
    types
        .iter()
        .find(|t| t.as_str() == "image/png")
        .or_else(|| types.iter().find(picture))
        .map(String::as_str)
}

/// Where files dropped onto the window go.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DropZone {
    /// The chat: its feed and message field. Files are attached to the message.
    Chat,
    /// The thread pane. Files are attached to the reply.
    Thread,
}

impl DropZone {
    /// The parent of the thread whose field takes the files, `None` for the chat's own field.
    pub fn thread(self, c: Ctx) -> Option<Timestamp> {
        match self {
            DropZone::Chat => None,
            DropZone::Thread => c.thread.get_untracked(),
        }
    }
}

/// The drop zone under the point `(x, y)` of the page, in CSS pixels; `None` for the rest of the
/// window (the sidebar, the headers), where a drop is ignored, as in Slack.
pub fn drop_zone(x: f64, y: f64) -> Option<DropZone> {
    let el = document().element_from_point(x as f32, y as f32)?;
    if el.closest(".thread-pane").ok().flatten().is_some_and(|p| has_field(&p)) {
        return Some(DropZone::Thread);
    }
    el.closest("main.main")
        .ok()
        .flatten()
        .filter(has_field)
        .map(|_| DropZone::Chat)
}

/// Whether the area has a message field: the thread pane has none while it shows a profile.
fn has_field(area: &web_sys::Element) -> bool {
    area.query_selector(".composer-wrap").ok().flatten().is_some()
}

/// [`drop_zone`] for a point in logical pixels of the window, as the app reports a drag from a
/// file manager: CSS pixels times the zoom. Not `devicePixelRatio`: it holds the screen's scale too.
pub fn drop_zone_logical(c: Ctx, x: f64, y: f64) -> Option<DropZone> {
    let zoom = (f64::from(c.settings.with_untracked(|s| s.zoom)) / 100.0).max(0.1);
    drop_zone(x / zoom, y / zoom)
}

/// Frames the chat or the thread pane the files dragged over it will be attached to; `None` takes
/// the frame away.
pub fn highlight_drop(zone: Option<DropZone>) {
    let frame = |selector: &str, on: bool| {
        if let Ok(list) = document().query_selector_all(selector) {
            for el in (0..list.length()).filter_map(|i| list.get(i)?.dyn_into::<web_sys::Element>().ok()) {
                let _ = el.class_list().toggle_with_force("drop-target", on && has_field(&el));
            }
        }
    };
    frame("main.main", zone == Some(DropZone::Chat));
    frame(".thread-pane", zone == Some(DropZone::Thread));
}

/// Drag and drop onto the chat: the files that reach the page (a picture dragged out of another
/// program) are attached to the message. Files from a file manager do not get here: the app takes
/// them itself and sends `mostik://files`.
pub fn attach_dropped(c: Ctx) {
    let has_files = |e: &web_sys::DragEvent| {
        e.data_transfer()
            .is_some_and(|d| d.types().includes(&"Files".into(), 0))
    };
    // Over the chat or the thread pane the drop is taken; elsewhere the browser shows that it is not.
    scoped_window_listener(ev::dragover, move |e| {
        if !has_files(&e) {
            return;
        }
        let zone = drop_zone(e.client_x().into(), e.client_y().into());
        highlight_drop(zone);
        if zone.is_some() {
            e.prevent_default();
        }
    });
    // Leaving the window: no element takes the drag next.
    scoped_window_listener(ev::dragleave, |e| {
        if e.related_target().is_none() {
            highlight_drop(None);
        }
    });
    scoped_window_listener(ev::drop, move |e| {
        highlight_drop(None);
        let Some(zone) = drop_zone(e.client_x().into(), e.client_y().into()) else {
            return;
        };
        if attach(c, e.data_transfer(), zone.thread(c)) {
            e.prevent_default();
        }
    });
}

/// The name a pasted file goes with: its own, or `image.png` and the like for a picture from the
/// clipboard, which may have none.
fn file_name(name: &str, kind: &str) -> String {
    if !name.trim().is_empty() {
        return name.to_owned();
    }
    let (base, sub) = kind.split_once('/').unwrap_or_default();
    let base = if base == "image" { "image" } else { "file" };
    match sub.split(['+', ';']).next().filter(|e| !e.is_empty()) {
        Some(ext) => format!("{base}.{ext}"),
        None => base.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_files_get_a_name() {
        assert_eq!(file_name("shot.png", "image/png"), "shot.png");
        assert_eq!(file_name("", "image/png"), "image.png");
        assert_eq!(file_name("", "image/svg+xml"), "image.svg");
        assert_eq!(file_name("", "application/pdf"), "file.pdf");
        assert_eq!(file_name("", ""), "file");
    }

    #[test]
    fn a_clipboard_picture_is_taken_as_png_first() {
        let types = |list: &[&str]| list.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
        assert_eq!(picture_type(&types(&["image/jpeg", "image/png"])), Some("image/png"));
        assert_eq!(picture_type(&types(&["text/plain", "image/webp"])), Some("image/webp"));
        assert_eq!(picture_type(&types(&["text/plain", "text/html"])), None);
    }
}
