//! Window state shared by all components (`Ctx`) and the types it holds, with the helpers around
//! it: the native menu check, the stored widths and drafts, the settings put on the document.

use std::collections::{HashSet, VecDeque};

use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::i18n::Lang;
use crate::settings::Settings;
use crate::{bridge, folders, hidden, ops, order, storage, time};

use super::composer::EditDraft;
use super::preferences::PrefTab;

/// Window state shared by all components.
#[derive(Clone, Copy)]
pub struct Ctx {
    pub snap: RwSignal<Snapshot>,
    pub current: RwSignal<ChatId>,
    pub thread: RwSignal<Option<Timestamp>>,
    pub collapsed: RwSignal<HashSet<String>>,
    /// "Find a conversation" above the sidebar sections: while it has text, the sections show
    /// only the conversations whose name has it.
    pub sb_query: RwSignal<String>,
    pub about: RwSignal<bool>,
    /// New version of Mostik: the "Update" button in the top bar.
    pub update: RwSignal<bridge::UpdateStatus>,
    /// `true` means data from Telemost (in Tauri), `false` means demo data in a browser.
    pub live: bool,
    pub phase: RwSignal<Phase>,
    pub online: RwSignal<bool>,
    /// `(chat, who, until when in ms)`.
    pub typing: RwSignal<Vec<(ChatId, UserId, f64)>>,
    pub last_typing: RwSignal<f64>,
    pub exhausted: RwSignal<HashSet<ChatId>>,
    pub loading_older: RwSignal<bool>,
    pub editing: RwSignal<Option<Message>>,
    /// The text being edited, outside the editor: a message line built again (the message list
    /// shown again after the pinned message, the text changed on another device) picks the
    /// draft up again.
    pub edit_draft: StoredValue<Option<EditDraft>>,
    pub confirm_delete: RwSignal<Option<Message>>,
    /// Channel the user is about to leave from the sidebar menu: waits for confirmation.
    pub confirm_leave: RwSignal<Option<ChatId>>,
    pub picker: RwSignal<Option<(ChatId, Timestamp)>>,
    /// Where to open the reaction picker: the top right corner of the clicked button.
    pub picker_at: RwSignal<(f64, f64)>,
    /// Right-click menu (as in Slack, at the cursor).
    pub ctx_menu: RwSignal<Option<CtxMenu>>,
    pub error: RwSignal<Option<String>>,
    pub notice: RwSignal<Option<String>>,
    pub lightbox: RwSignal<Option<Attachment>>,
    pub me_menu: RwSignal<bool>,
    /// Selected organization (if there are several): the sidebar shows its chats and direct messages.
    pub org: RwSignal<Option<u64>>,
    /// Chat navigation history for the "Back" and "Forward" buttons.
    pub nav_back: RwSignal<Vec<ChatId>>,
    pub nav_forward: RwSignal<Vec<ChatId>>,
    /// Recently opened chats, for the "History" button.
    pub recent: RwSignal<Vec<ChatId>>,
    /// Message reached from search or a link: scroll to it and highlight it.
    pub highlight: RwSignal<Option<(ChatId, Timestamp)>>,
    /// Chat marked unread while you are in it: do not mark it read until you leave it.
    pub hold_unread: RwSignal<Option<ChatId>>,
    /// Message being forwarded (the "Forward message" dialog).
    pub forwarding: RwSignal<Option<Message>>,
    /// Message the composer of its chat or thread replies to: the bar above the field.
    pub replying: RwSignal<Option<ops::ReplyTarget>>,
    /// The "Pinned" tab in the chat header is open.
    pub pins_tab: RwSignal<bool>,
    /// The "Browse channels" dialog.
    pub browsing: RwSignal<bool>,
    pub settings: RwSignal<Settings>,
    pub status_dialog: RwSignal<bool>,
    /// Files attached to the main composer (picked in the dialog or dropped).
    pub pending_files: RwSignal<Vec<bridge::PendingFile>>,
    pub new_message: RwSignal<bool>,
    /// The "Create a channel" dialog.
    pub creating_channel: RwSignal<Option<NewChannel>>,
    pub details: RwSignal<Option<DetailsTab>>,
    /// What is selected in the rail and sidebar: home, direct messages or "Drafts & sent".
    pub view: RwSignal<ViewMode>,
    /// Per-chat drafts, as in Slack; kept between runs.
    pub drafts: RwSignal<std::collections::HashMap<ChatId, String>>,
    /// Profile open in the right panel and the loaded profiles.
    pub profile: RwSignal<Option<UserId>>,
    pub profiles: RwSignal<std::collections::HashMap<UserId, UserProfile>>,
    pub pref_tab: RwSignal<PrefTab>,
    /// Widths the user drags with the mouse (as in Slack), in pixels.
    pub sidebar_w: RwSignal<f64>,
    pub thread_w: RwSignal<f64>,
    pub dragging: RwSignal<Option<Drag>>,
    /// Custom chat order in the sidebar sections, kept between runs.
    pub chat_order: RwSignal<order::ChatOrder>,
    /// A sidebar chat being dragged to a new place.
    pub chat_drag: RwSignal<Option<ChatDrag>>,
    pub prefs: RwSignal<bool>,
    pub search: RwSignal<String>,
    /// Ticks once a second, to expire "is typing…".
    pub now: RwSignal<f64>,
    /// The call window that is open now, shown at the bottom of the sidebar like a Slack huddle.
    pub call: RwSignal<Option<bridge::CallInfo>>,
    /// A call ringing for the user: the incoming call dialog shows while it is set.
    pub ring: RwSignal<Option<bridge::Ring>>,
    /// Calls going on in chats (from their call notes), with the meeting links.
    pub active_calls: Memo<std::collections::BTreeMap<ChatId, String>>,
    /// The latest call of every chat Telemost reported since the connection, with its start and
    /// the number of people in it.
    pub meetings: RwSignal<std::collections::HashMap<ChatId, Meeting>>,
    /// Followed threads with their unread replies, newest first.
    pub threads: RwSignal<Vec<FollowedThread>>,
    /// People picked from `@` suggestions or named in an edited message, by the name the
    /// text shows: sending turns `@Name` into their `@<guid>` even when the name is not unique.
    pub picked: RwSignal<std::collections::HashMap<String, UserId>>,
    /// The latest messages removed as deleted, so a deletion is counted once (see `ops::snapshot::remove`).
    pub deleted: RwSignal<VecDeque<(ChatId, Timestamp)>>,
    /// Chats hidden from the sidebar with "×", kept between runs.
    pub hidden: RwSignal<hidden::Hidden>,
    /// Own sidebar sections: the Telemost chat folders of the current organization.
    pub folders: Memo<folders::Folders>,
    /// The sidebar sections: the Telemost ones with the folders.
    pub sections: Memo<Vec<Section>>,
    /// The "New section" / "Rename section" dialog.
    pub folder_edit: RwSignal<Option<FolderEdit>>,
    /// Custom emoji of the user's organizations.
    pub custom: RwSignal<Vec<mostik_core::custom_emoji::EmojiSet>>,
    /// The "Add custom emoji" dialog, opened from the emoji picker as in Slack.
    pub adding_emoji: RwSignal<bool>,
}

/// What the section name dialog does.
#[derive(Clone, PartialEq)]
pub enum FolderEdit {
    /// Creates a section, moving the chat into it.
    New(Option<ChatId>),
    Rename(String),
}

/// A sidebar chat held with the mouse button.
#[derive(Clone, PartialEq)]
pub struct ChatDrag {
    pub section: String,
    pub chat: ChatId,
    /// Where the button went down.
    pub from: (f64, f64),
    /// The pointer moved far enough: it is a drag, not a click.
    pub active: bool,
    /// Where the chat would go.
    pub target: Option<DropAt>,
    /// Already dropped: the click that follows the button release must not open the chat.
    pub dropped: bool,
}

/// Drop place of a dragged chat: a section and, among its rows, next to this chat (after it with
/// `true`); an empty section has no rows.
#[derive(Clone, PartialEq)]
pub struct DropAt {
    pub section: String,
    pub anchor: Option<(ChatId, bool)>,
}

/// Where "Create a channel" was started: from one's own section, the new channel goes into it.
#[derive(Clone, Default, PartialEq)]
pub struct NewChannel {
    pub folder: Option<String>,
}

/// Open right-click menu: the point that was clicked and what for.
#[derive(Clone)]
pub struct CtxMenu {
    pub x: f64,
    pub y: f64,
    pub target: CtxTarget,
}

#[derive(Clone)]
pub enum CtxTarget {
    Message {
        msg: Box<Message>,
        mine: bool,
        /// Text selected in the message when the menu opened: "Quote selection" sends it.
        quote: Option<String>,
    },
    Chat(ChatId),
    /// "Sidebar settings" menu at the organization name.
    SidebarPrefs,
    /// A person: right click on an author's name or avatar.
    User(UserId),
    /// Sidebar section header.
    Section(String),
    /// Organization menu: the name in the sidebar header.
    Workspace,
    /// Organization switcher: the tile in the rail.
    Organizations,
    /// "Add channels": create or browse.
    AddChannels,
}

impl CtxMenu {
    pub(super) fn is_message(&self, key: &(ChatId, Timestamp)) -> bool {
        matches!(&self.target, CtxTarget::Message { msg, .. } if (&msg.chat_id, msg.ts) == (&key.0, key.1))
    }

    pub(super) fn is_chat(&self, id: &ChatId) -> bool {
        matches!(&self.target, CtxTarget::Chat(chat) if chat == id)
    }
}

/// A right click that needs the system editing menu rather than the UI menu:
/// in a text field, on selected text, on a link or an image. Slack works the same way.
pub(super) fn wants_native_menu(e: &ev::MouseEvent) -> bool {
    let selected = window()
        .get_selection()
        .ok()
        .flatten()
        .is_some_and(|s| !s.is_collapsed());
    let editable = e
        .target()
        .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
        .and_then(|el| {
            el.closest("input, textarea, [contenteditable], a[href], .images img, .lightbox img")
                .ok()
                .flatten()
        })
        .is_some();
    selected || editable
}

/// Text selected within the text of message `ts` (one message, not a forwarded one inside it):
/// what "Quote selection" quotes.
pub(super) fn selected_in_message(ts: Timestamp) -> Option<String> {
    use wasm_bindgen::JsCast;
    let sel = window().get_selection().ok().flatten()?;
    if sel.is_collapsed() {
        return None;
    }
    let text_of = |node: Option<web_sys::Node>| -> Option<web_sys::Element> {
        let node = node?;
        let el = match node.dyn_ref::<web_sys::Element>() {
            Some(el) => el.clone(),
            None => node.parent_element()?,
        };
        let text = el.closest(".msg-text").ok().flatten()?;
        let row = text.closest(".msg").ok().flatten()?;
        let own = text.closest(".fwd, .reply-ref").ok().flatten().is_none();
        (own && row.get_attribute("data-ts") == Some(ts.to_string())).then_some(text)
    };
    let start = text_of(sel.anchor_node())?;
    let end = text_of(sel.focus_node())?;
    if !start.is_same_node(Some(&end)) {
        return None;
    }
    let quote = String::from(sel.to_string()).trim().to_owned();
    (!quote.is_empty()).then_some(quote)
}

/// Outside the app (`scripts/ui.sh serve`, a build with the `demo` feature): the demo data,
/// with `#dev` open.
#[cfg(feature = "demo")]
pub(super) fn show_demo(c: Ctx) {
    let mut demo = mostik_core::mock::demo((js_sys::Date::now() * 1000.0) as Timestamp);
    if let Some(ch) = demo.chats.iter_mut().find(|ch| ch.id.0 == "dev") {
        ch.unread = 0;
        ch.mentions = 0;
    }
    c.snap.set(demo);
    c.phase.set(Phase::Ready);
    ops::load_threads(c);
    ops::open_chat(c, ChatId("dev".into()));
}

/// Closes the right-click menu.
pub(super) fn close_menus(c: Ctx) {
    c.ctx_menu.set(None);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Drag {
    Sidebar,
    Thread,
}

pub(super) const RAIL_W: f64 = 70.0;
pub(super) const SIDEBAR_RANGE: (f64, f64) = (200.0, 480.0);
pub(super) const THREAD_MIN: f64 = 320.0;
pub(super) const MAIN_MIN: f64 = 380.0;

pub(super) fn stored_width(key: &str, default: f64) -> f64 {
    storage::get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub(super) fn store_width(key: &str, value: f64) {
    storage::set(key, &format!("{value:.0}"));
}

/// The language and clock format of the strings and times built from now on.
pub(super) fn set_locale(lang: Lang, clock24: bool) {
    crate::i18n::set_current(lang);
    time::configure(lang, clock24);
}

/// Language, time format, theme and colour mode go onto the document root.
pub(super) fn apply_settings(st: &Settings) {
    set_locale(st.lang, st.clock24);
    if let Some(root) = document().document_element() {
        let _ = root.set_attribute("lang", st.lang.code());
        let _ = root.set_attribute("data-mode", if st.dark() { "dark" } else { "light" });
        let _ = root.set_attribute("data-theme", &st.theme);
    }
    // Zoom: in the app it is the webview zoom (mouse coordinates stay correct), in the demo it is CSS zoom.
    let zoom = st.zoom;
    if bridge::available() {
        spawn_local(async move {
            if let Err(e) = bridge::set_zoom(zoom).await {
                leptos::logging::warn!("zoom: {e}");
            }
        });
    } else if let Some(root) = document().document_element() {
        let _ = root.set_attribute("style", &format!("zoom: {}%", zoom));
    }
}

/// Enter sends the message (otherwise Ctrl+Enter), as in "Preferences → Advanced".
pub fn enter_sends() -> bool {
    use_context::<Ctx>().is_none_or(|c| c.settings.with_untracked(|s| s.enter_sends))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DetailsTab {
    About,
    Members,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Home,
    Dms,
    Drafts,
    /// Followed threads, like Slack's "Threads".
    Threads,
    /// "Mentions & reactions", like Activity in Slack.
    Activity,
}

impl ViewMode {
    /// A page of the home view in place of a chat: "Threads" or "Drafts & sent".
    pub fn is_home_page(self) -> bool {
        matches!(self, ViewMode::Drafts | ViewMode::Threads)
    }
}

const DRAFTS_KEY: &str = "mostik.drafts";

pub(super) fn load_drafts() -> std::collections::HashMap<ChatId, String> {
    storage::load::<Vec<(ChatId, String)>>(DRAFTS_KEY)
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

pub(super) fn save_drafts(d: &std::collections::HashMap<ChatId, String>) {
    storage::save(DRAFTS_KEY, &d.iter().collect::<Vec<_>>());
}

#[derive(Clone, PartialEq)]
pub enum Phase {
    Loading(&'static str),
    Login { reason: Option<String> },
    Ready,
}

pub(super) fn ctx() -> Ctx {
    expect_context::<Ctx>()
}

pub(super) fn empty_snapshot() -> Snapshot {
    Snapshot {
        me: UserId(String::new()),
        organization: String::new(),
        organization_avatar: None,
        organizations: vec![],
        users: vec![],
        chats: vec![],
        sections: vec![],
        messages: vec![],
        folders: vec![],
        paused: None,
    }
}
