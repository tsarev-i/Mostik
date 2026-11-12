use std::collections::HashSet;
use std::time::Duration;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::markup::{Inline, MENTION_ALL, Me, Mentionable, parse_inline};
use mostik_core::*;

use crate::i18n::{Lang, tr};
use crate::icons::{I, Icon};
use crate::settings::{ColorMode, NotifyMode, OWN_THEMES, Settings, SidebarFilter, SidebarSort, THEMES};
use crate::{bridge, folders, hidden, ops, order, time};

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
    pub menu: RwSignal<Option<(ChatId, Timestamp)>>,
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
    /// Messages already removed as deleted, so a deletion is counted once.
    pub deleted: RwSignal<HashSet<(ChatId, Timestamp)>>,
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
    fn is_message(&self, key: &(ChatId, Timestamp)) -> bool {
        matches!(&self.target, CtxTarget::Message { msg, .. } if (&msg.chat_id, msg.ts) == (&key.0, key.1))
    }

    fn is_chat(&self, id: &ChatId) -> bool {
        matches!(&self.target, CtxTarget::Chat(chat) if chat == id)
    }
}

/// A right click that needs the system editing menu rather than the UI menu:
/// in a text field, on selected text, on a link or an image. Slack works the same way.
fn wants_native_menu(e: &ev::MouseEvent) -> bool {
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
fn selected_in_message(ts: Timestamp) -> Option<String> {
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

/// Closes the message menu and the right-click menu.
fn close_menus(c: Ctx) {
    c.menu.set(None);
    c.ctx_menu.set(None);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Drag {
    Sidebar,
    Thread,
}

const RAIL_W: f64 = 70.0;
const SIDEBAR_RANGE: (f64, f64) = (200.0, 480.0);
const THREAD_MIN: f64 = 320.0;
const MAIN_MIN: f64 = 380.0;

fn stored_width(key: &str, default: f64) -> f64 {
    window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(key).ok().flatten())
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn store_width(key: &str, value: f64) {
    if let Some(s) = window().local_storage().ok().flatten() {
        let _ = s.set_item(key, &format!("{value:.0}"));
    }
}

/// Language, time format, theme and colour mode go onto the document root.
fn apply_settings(st: &Settings) {
    crate::i18n::set_current(st.lang);
    time::configure(st.lang, st.clock24);
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

fn load_drafts() -> std::collections::HashMap<ChatId, String> {
    window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(DRAFTS_KEY).ok().flatten())
        .and_then(|v| serde_json::from_str::<Vec<(ChatId, String)>>(&v).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

fn save_drafts(d: &std::collections::HashMap<ChatId, String>) {
    if let (Some(s), Ok(v)) = (
        window().local_storage().ok().flatten(),
        serde_json::to_string(&d.iter().collect::<Vec<_>>()),
    ) {
        let _ = s.set_item(DRAFTS_KEY, &v);
    }
}

#[derive(Clone, PartialEq)]
pub enum Phase {
    Loading(&'static str),
    Login { reason: Option<String> },
    Ready,
}

fn ctx() -> Ctx {
    expect_context::<Ctx>()
}

fn empty_snapshot() -> Snapshot {
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

#[component]
pub fn App() -> impl IntoView {
    let live = bridge::available();
    let snap = RwSignal::new(empty_snapshot());
    let org = RwSignal::new(
        window()
            .local_storage()
            .ok()
            .flatten()
            .and_then(|s| s.get_item(ORG_KEY).ok().flatten())
            .and_then(|v| v.parse().ok()),
    );
    let folder_list = Memo::new(move |_| {
        let chosen = org.get();
        snap.with(|s| {
            let key = org_key(s, chosen);
            s.folders
                .iter()
                .filter(|f| f.org == key)
                .map(folders::from_telemost)
                .collect::<folders::Folders>()
        })
    });
    let settings = RwSignal::new(Settings::load());
    let c = Ctx {
        snap,
        current: RwSignal::new(ChatId(String::new())),
        thread: RwSignal::new(None),
        collapsed: RwSignal::new(HashSet::new()),
        sb_query: RwSignal::new(String::new()),
        about: RwSignal::new(false),
        update: RwSignal::new(bridge::UpdateStatus::None),
        live,
        phase: RwSignal::new(Phase::Loading(tr().connecting)),
        online: RwSignal::new(true),
        typing: RwSignal::new(vec![]),
        last_typing: RwSignal::new(0.0),
        exhausted: RwSignal::new(HashSet::new()),
        loading_older: RwSignal::new(false),
        editing: RwSignal::new(None),
        edit_draft: StoredValue::new(None),
        confirm_delete: RwSignal::new(None),
        confirm_leave: RwSignal::new(None),
        picker: RwSignal::new(None),
        picker_at: RwSignal::new((0.0, 0.0)),
        menu: RwSignal::new(None),
        ctx_menu: RwSignal::new(None),
        error: RwSignal::new(None),
        notice: RwSignal::new(None),
        lightbox: RwSignal::new(None),
        me_menu: RwSignal::new(false),
        org,
        nav_back: RwSignal::new(vec![]),
        nav_forward: RwSignal::new(vec![]),
        recent: RwSignal::new(vec![]),
        highlight: RwSignal::new(None),
        hold_unread: RwSignal::new(None),
        forwarding: RwSignal::new(None),
        replying: RwSignal::new(None),
        pins_tab: RwSignal::new(false),
        browsing: RwSignal::new(false),
        settings,
        status_dialog: RwSignal::new(false),
        pending_files: RwSignal::new(vec![]),
        new_message: RwSignal::new(false),
        creating_channel: RwSignal::new(None),
        details: RwSignal::new(None),
        view: RwSignal::new(ViewMode::Home),
        drafts: RwSignal::new(load_drafts()),
        profile: RwSignal::new(None),
        profiles: RwSignal::new(std::collections::HashMap::new()),
        pref_tab: RwSignal::new(PrefTab::Notifications),
        sidebar_w: RwSignal::new(stored_width("mostik.sidebar_w", 270.0)),
        thread_w: RwSignal::new(stored_width("mostik.thread_w", 400.0)),
        dragging: RwSignal::new(None),
        chat_order: RwSignal::new(order::load()),
        chat_drag: RwSignal::new(None),
        prefs: RwSignal::new(false),
        search: RwSignal::new(String::new()),
        now: RwSignal::new(js_sys::Date::now()),
        call: RwSignal::new(None),
        ring: RwSignal::new(None),
        active_calls: Memo::new(move |_| snap.with(Snapshot::active_calls)),
        meetings: RwSignal::new(Default::default()),
        threads: RwSignal::new(vec![]),
        picked: RwSignal::new(Default::default()),
        deleted: RwSignal::new(Default::default()),
        hidden: RwSignal::new(hidden::load()),
        folders: folder_list,
        sections: Memo::new(move |_| {
            let apart = settings.with(|s| s.group_calls_apart);
            snap.with(|s| folder_list.with(|f| folders::arrange(&s.sections, f, apart)))
        }),
        folder_edit: RwSignal::new(None),
        custom: RwSignal::new(vec![]),
        adding_emoji: RwSignal::new(false),
    };
    provide_context(c);
    apply_settings(&c.settings.get_untracked());
    // A draft lives only while its message is being edited.
    Effect::new(move |_| {
        let key = c.editing.with(|e| e.as_ref().map(|m| (m.chat_id.clone(), m.ts)));
        c.edit_draft.update_value(|d| {
            if d.as_ref().map(|d| &d.key) != key.as_ref() {
                *d = None;
            }
        });
    });
    // Preferences: save, apply the language, time format and theme, and pass them to the backend.
    Effect::new(move |_| {
        let st = c.settings.get();
        st.save();
        apply_settings(&st);
        if c.live {
            spawn_local(async move {
                let _ = bridge::set_preferences(&st).await;
                let _ = bridge::set_auto_update(st.auto_update).await;
            });
        }
    });
    set_interval(move || c.now.set(js_sys::Date::now()), Duration::from_secs(1));
    // A notification pause ends by itself: Telemost says nothing then.
    Effect::new(move |_| {
        let now = (c.now.get() * 1000.0) as Timestamp;
        if c.snap.with_untracked(|s| s.paused.is_some_and(|p| !p.active_at(now))) {
            c.snap.update(|s| {
                s.paused = None;
                let me = s.me.clone();
                if let Some(u) = s.users.iter_mut().find(|u| u.id == me)
                    && u.availability == Availability::DoNotDisturb
                {
                    u.availability = Availability::Default;
                }
            });
        }
    });
    // Colleagues' presence: the subscription lives 10 minutes and is renewed a minute before it
    // ends; meanwhile changes come as pushes. Leaving is not pushed: a person whose last
    // `Heartbeat` is too old turns gray locally.
    set_interval(
        move || {
            if c.phase.get_untracked() == Phase::Ready {
                ops::refresh_presence(c)
            }
        },
        Duration::from_secs(9 * 60),
    );
    set_interval(move || ops::expire_presence(c), Duration::from_secs(60));
    Effect::new(move |_| c.drafts.with(save_drafts));
    Effect::new(move |_| c.chat_order.with(order::save));
    Effect::new(move |_| c.hidden.with(hidden::save));
    // Sections an earlier Mostik kept in this browser move into Telemost folders, once.
    let migrated = StoredValue::new(false);
    Effect::new(move |_| {
        if !c.live || c.phase.get() != Phase::Ready || migrated.get_value() {
            return;
        }
        migrated.set_value(true);
        let local = folders::take_local();
        if !local.is_empty() {
            ops::change_folders(c, move |list| {
                for f in local {
                    match list.iter_mut().find(|x| x.name == f.name) {
                        Some(x) => {
                            for id in f.chats {
                                if !x.chats.contains(&id) {
                                    x.chats.push(id);
                                }
                            }
                        }
                        None => list.push(folders::Folder {
                            id: folders::new_id(list, ops::random_u64()),
                            ..f
                        }),
                    }
                }
            });
        }
    });
    // A hidden chat that got something new is an ordinary chat again.
    Effect::new(move |_| {
        if c.hidden.with(|h| h.is_empty()) {
            return;
        }
        let back: Vec<ChatId> = c.snap.with(|s| {
            c.hidden.with_untracked(|h| {
                h.iter()
                    .filter(|(id, at)| s.chat(id).is_some_and(|ch| hidden::came_back(ch, **at)))
                    .map(|(id, _)| id.clone())
                    .collect()
            })
        });
        if !back.is_empty() {
            c.hidden.update(|h| h.retain(|id, _| !back.contains(id)));
        }
    });

    // Unread count in the window title, as in Slack.
    Effect::new(move |_| {
        let unread: u32 = c
            .snap
            .with(|s| s.chats.iter().filter(|ch| !ch.muted).map(|ch| ch.unread.min(1)).sum());
        c.settings.with(|s| s.lang);
        let title = tr().window_title(unread);
        document().set_title(&title);
        if bridge::available() {
            spawn_local(async move {
                let _ = bridge::set_window_title(&title).await;
            });
        }
    });

    // Jump to a message: scroll to it (once rendered) and remove the highlight after a few seconds.
    Effect::new(move |_| {
        let Some((_, ts)) = c.highlight.get() else { return };
        fn scroll_to(ts: Timestamp, tries: u32) {
            let found = document()
                .query_selector_all(&format!(".msg[data-ts=\"{ts}\"]"))
                .ok()
                .and_then(|list| list.item(list.length().saturating_sub(1)))
                .and_then(|n| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(n).ok());
            match found {
                Some(el) => {
                    let opts = web_sys::ScrollIntoViewOptions::new();
                    opts.set_block(web_sys::ScrollLogicalPosition::Center);
                    el.scroll_into_view_with_scroll_into_view_options(&opts);
                }
                None if tries < 25 => set_timeout(move || scroll_to(ts, tries + 1), Duration::from_millis(200)),
                None => {}
            }
        }
        request_animation_frame(move || scroll_to(ts, 0));
        set_timeout(
            move || {
                if c.highlight.get_untracked().is_some_and(|(_, t)| t == ts) {
                    c.highlight.set(None)
                }
            },
            Duration::from_secs(4),
        );
    });

    // Errors are shown as a toast for 6 seconds.
    Effect::new(move |_| {
        if c.error.get().is_some() {
            set_timeout(move || c.error.set(None), Duration::from_secs(6));
        }
    });

    Effect::new(move |_| {
        if c.notice.get().is_some() {
            set_timeout(move || c.notice.set(None), Duration::from_secs(5));
        }
    });

    // Dragging the sidebar and thread borders.
    let drag_to = move |x: f64| {
        let Some(drag) = c.dragging.get_untracked() else { return };
        let total = window().inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1280.0);
        match drag {
            Drag::Sidebar => c.sidebar_w.set((x - RAIL_W).clamp(SIDEBAR_RANGE.0, SIDEBAR_RANGE.1)),
            Drag::Thread => {
                let max = (total - RAIL_W - c.sidebar_w.get_untracked() - MAIN_MIN).max(THREAD_MIN);
                c.thread_w.set((total - x - 6.0).clamp(THREAD_MIN, max));
            }
        }
    };
    let drag_end = move || {
        if let Some(drag) = c.dragging.get_untracked() {
            c.dragging.set(None);
            match drag {
                Drag::Sidebar => store_width("mostik.sidebar_w", c.sidebar_w.get_untracked()),
                Drag::Thread => store_width("mostik.thread_w", c.thread_w.get_untracked()),
            }
        }
    };
    // Listen to both pointer and mouse events: some environments send only one kind.
    scoped_window_listener(ev::pointermove, move |e| {
        drag_to(e.client_x() as f64);
        chat_drag_to(c, &e);
    });
    scoped_window_listener(ev::mousemove, move |e| {
        drag_to(e.client_x() as f64);
        chat_drag_to(c, &e);
    });
    scoped_window_listener(ev::pointerup, move |_| {
        drag_end();
        chat_drop(c);
    });
    scoped_window_listener(ev::mouseup, move |_| {
        drag_end();
        chat_drop(c);
    });

    // Ctrl+= / Ctrl+- / Ctrl+0 zoom, as in Slack.
    scoped_window_listener(ev::keydown, move |e| {
        if !(e.ctrl_key() || e.meta_key()) || e.alt_key() {
            return;
        }
        let next = match e.key().as_str() {
            "=" | "+" => Some(true),
            "-" | "_" => Some(false),
            "0" => None,
            _ => return,
        };
        e.prevent_default();
        c.settings
            .update(|s| s.zoom = next.map_or(100, |up| crate::settings::zoom_step(s.zoom, up)));
    });

    // Esc, as in Slack: first closes an open menu or image viewer, otherwise the topmost dialog.
    scoped_window_listener(ev::keydown, move |e| {
        if e.key() != "Escape" {
            return;
        }
        // A ringing call is answered with its buttons; Esc must not close a dialog under it.
        if c.ring.with_untracked(Option::is_some) {
            e.prevent_default();
            return;
        }
        let popup = c.menu.get_untracked().is_some()
            || c.ctx_menu.with_untracked(Option::is_some)
            || c.picker.get_untracked().is_some()
            || c.lightbox.with_untracked(Option::is_some)
            || c.me_menu.get_untracked();
        if popup {
            c.menu.set(None);
            c.ctx_menu.set(None);
            c.picker.set(None);
            c.lightbox.set(None);
            c.me_menu.set(false);
            return;
        }
        // An emoji picker or a nested dialog closes itself, leaving the dialog under it open.
        if document()
            .query_selector(".emoji-picker, .modal-backdrop.nested")
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        if c.adding_emoji.get_untracked() {
            c.adding_emoji.set(false);
        } else if c.confirm_delete.with_untracked(Option::is_some) {
            c.confirm_delete.set(None);
        } else if c.confirm_leave.with_untracked(Option::is_some) {
            c.confirm_leave.set(None);
        } else if c.folder_edit.with_untracked(Option::is_some) {
            c.folder_edit.set(None);
        } else if c.browsing.get_untracked() {
            c.browsing.set(false);
        } else if c.forwarding.with_untracked(Option::is_some) {
            c.forwarding.set(None);
        } else if c.status_dialog.get_untracked() {
            c.status_dialog.set(false);
        } else if c.new_message.get_untracked() {
            c.new_message.set(false);
        } else if c.creating_channel.get_untracked().is_some() {
            c.creating_channel.set(None);
        } else if c.details.get_untracked().is_some() {
            c.details.set(None);
        } else if c.prefs.get_untracked() {
            c.prefs.set(false);
        } else if c.about.get_untracked() {
            c.about.set(false);
        } else if c.replying.with_untracked(Option::is_some) && c.editing.with_untracked(Option::is_none) {
            c.replying.set(None);
        } else {
            return;
        }
        e.prevent_default();
    });

    // The right-click menu closes on a click outside it, on scroll and when the window loses focus.
    scoped_window_listener(ev::mousedown, move |e| {
        let inside = |sel: &str| {
            e.target()
                .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
                .and_then(|el| el.closest(sel).ok().flatten())
                .is_some()
        };
        if !inside(".ctx-menu") {
            c.ctx_menu.set(None);
        }
        // The avatar menu too; its own button toggles it.
        if c.me_menu.get_untracked() && !inside(".me-menu, .rail-me") {
            c.me_menu.set(false);
        }
    });
    scoped_window_listener(ev::wheel, move |_| c.ctx_menu.set(None));
    scoped_window_listener(ev::blur, move |_| c.ctx_menu.set(None));
    scoped_window_listener(ev::focus, move |_| {
        ops::refresh_me(c);
        ops::recount_soon(c);
    });
    scoped_window_listener(ev::resize, move |_| c.ctx_menu.set(None));

    if live {
        ops::subscribe(c);
        spawn_local(async move { ops::apply_boot(c, bridge::bootstrap().await) });
    } else {
        let mut demo = mostik_core::mock::demo();
        if let Some(ch) = demo.chats.iter_mut().find(|ch| ch.id.0 == "dev") {
            ch.unread = 0;
            ch.mentions = 0;
        }
        c.snap.set(demo);
        c.phase.set(Phase::Ready);
        ops::load_threads(c);
        ops::open_chat(c, ChatId("dev".into()));
    }
    // Only these two settings rebuild the whole view; the others are read where they apply.
    let lang_clock = Memo::new(move |_| c.settings.with(|s| (s.lang, s.clock24)));
    view! {
        {move || {
            // Language and time format before building the view: effects run only after rendering.
            let (lang, clock24) = lang_clock.get();
            crate::i18n::set_current(lang);
            time::configure(lang, clock24);
            let phase = c.phase.get();
            match phase {
            Phase::Ready => view! { <Workspace/> }.into_any(),
            Phase::Loading(text) => view! { <Splash text=text/> }.into_any(),
            Phase::Login { reason } => view! { <LoginScreen reason=reason/> }.into_any(),
            }
        }}
        <HoverTip/>
        <Show when=move || c.about.get()>
            <AboutDialog/>
        </Show>
        {move || c.creating_channel.get().map(|to| view! { <CreateChannelDialog to=to/> })}
        {move || c.new_message.get().then(|| view! { <NewMessageDialog/> })}
        {move || c.forwarding.get().map(|m| view! { <ForwardDialog msg=m/> })}
        {move || c.browsing.get().then(|| view! { <BrowseChannelsDialog/> })}
        {move || c.status_dialog.get().then(|| view! { <StatusDialog/> })}
        {move || c.details.get().map(|tab| view! { <ChannelDetailsDialog initial=tab/> })}
        {move || {
            // Recreate the dialog when the language changes; the selected section is kept in `pref_tab`.
            crate::i18n::set_current(c.settings.with(|s| s.lang));
            c.prefs.get().then(|| view! { <PreferencesDialog/> })
        }}
        <Show when=move || c.confirm_delete.get().is_some()>
            <DeleteDialog/>
        </Show>
        {move || c.confirm_leave.get().map(|id| view! { <LeaveDialog id=id/> })}
        {move || c.folder_edit.get().map(|edit| view! { <FolderDialog edit=edit/> })}
        {move || c.adding_emoji.get().then(|| view! { <AddEmojiDialog/> })}
        <Show when=move || c.ring.with(Option::is_some)>
            <crate::ring::RingDialog ring=c.ring/>
        </Show>
        // Toasts stack at the bottom instead of covering each other.
        <div class="toasts">
            {move || c.error.get().map(|e| view! {
                <div class="toast" on:click=move |_| c.error.set(None)>{e}</div>
            })}
            {move || c.notice.get().map(|e| view! {
                <div class="toast ok" on:click=move |_| c.notice.set(None)>{e}</div>
            })}
        </div>
        {move || c.lightbox.get().map(|a| view! {
            <div class="lightbox" on:click=move |_| c.lightbox.set(None)>
                <img src=bridge::file_url(&a.file_id, None) alt=a.name.clone()/>
                <div class="lightbox-name">{a.name.clone()}</div>
            </div>
        })}
    }
}

#[component]
fn Splash(text: &'static str) -> impl IntoView {
    view! {
        <div class="window center">
            <div class="splash">
                <div class="spinner"></div>
                <p>{text}</p>
            </div>
        </div>
    }
}

#[component]
fn LoginScreen(reason: Option<String>) -> impl IntoView {
    let c = ctx();
    let start = move |_| {
        c.phase.set(Phase::Loading(tr().finish_login));
        spawn_local(async move { ops::apply_boot(c, bridge::login().await) });
    };
    view! {
        <div class="window center">
            <div class="login-card">
                <img class="login-logo" src="app-icon.png" alt=""/>
                <h1>{tr().app_name}</h1>
                <p class="login-sub">{tr().unofficial_client}</p>
                {reason.map(|r| view! { <p class="login-error">{r}</p> })}
                <button class="login-btn" on:click=start>{tr().login_button}</button>
                <p class="login-note">
                    {tr().login_note_password}
                </p>
                <p class="login-note">
                    {tr().login_note_unofficial}
                </p>
            </div>
        </div>
    }
}

#[component]
fn Workspace() -> impl IntoView {
    let c = ctx();
    let inner_width = || window().inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1280.0);
    let win_w = RwSignal::new(inner_width());
    scoped_window_listener(ev::resize, move |_| win_w.set(inner_width()));
    // The right pane gives way to the messages down to MAIN_MIN, then down to THREAD_MIN itself;
    // when even that does not fit (a narrow window), it takes the whole column in place of the chat.
    let pane_w = Memo::new(move |_| {
        // The card's margin and the two resizers.
        let room = win_w.get() - RAIL_W - c.sidebar_w.get() - 12.0;
        let wanted = c.thread_w.get();
        if room - wanted >= MAIN_MIN {
            Some(wanted)
        } else if room - MAIN_MIN >= THREAD_MIN {
            Some(room - MAIN_MIN)
        } else {
            None
        }
    });
    let pane_open = move || c.thread.get().is_some() || c.profile.get().is_some();
    view! {
        <div class=move || {
            if c.dragging.get().is_some() {
                "window dragging"
            } else if c.chat_drag.with(|d| d.as_ref().is_some_and(|d| d.active && !d.dropped)) {
                "window chat-dragging"
            } else {
                "window"
            }
        }>
            <TopBar/>
            <div class="body">
                <Rail/>
                <div class=move || if pane_open() && pane_w.get().is_none() { "card pane-full" } else { "card" }
                    style=move || pane_w.get().map(|w| format!("--pane-w:{w:.0}px")).unwrap_or_default()>
                    <Sidebar/>
                    <div class="resizer" title=tr().drag_to_resize
                        on:mousedown=move |e| { e.prevent_default(); c.dragging.set(Some(Drag::Sidebar)); }></div>
                    <main class="main">
                        <Show when=move || !c.online.get()>
                            <div class="offline-banner">{tr().offline}</div>
                        </Show>
                        {move || if c.view.get() == ViewMode::Drafts {
                            view! { <DraftsView/> }.into_any()
                        } else if c.view.get() == ViewMode::Threads {
                            view! { <ThreadsView/> }.into_any()
                        } else {
                            view! {
                                <ChannelHeader/>
                                {move || if c.pins_tab.get() {
                                    view! { <PinsView/> }.into_any()
                                } else {
                                    view! { <MessageList/><Composer/> }.into_any()
                                }}
                            }.into_any()
                        }}
                    </main>
                    <Show when=pane_open>
                        <div class="resizer pane-resizer" title=tr().drag_to_resize
                            on:mousedown=move |e| { e.prevent_default(); c.dragging.set(Some(Drag::Thread)); }></div>
                        {move || if c.profile.get().is_some() {
                            view! { <ProfilePane/> }.into_any()
                        } else {
                            view! { <ThreadPane/> }.into_any()
                        }}
                    </Show>
                </div>
            </div>
            {move || c.ctx_menu.get().map(|menu| view! { <ContextMenu menu=menu/> })}
        </div>
    }
}

/// Right-click menu. Opens at the cursor and never leaves the window.
#[component]
fn ContextMenu(menu: CtxMenu) -> impl IntoView {
    let c = ctx();
    let node = NodeRef::<html::Div>::new();
    let pos = RwSignal::new((menu.x, menu.y));
    Effect::new(move |_| {
        let Some(el) = node.get() else { return };
        let (w, h) = (el.offset_width() as f64, el.offset_height() as f64);
        let win = window();
        let vw = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(f64::MAX);
        let vh = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(f64::MAX);
        let x = if menu.x + w > vw - 4.0 {
            (menu.x - w).max(4.0)
        } else {
            menu.x
        };
        let y = if menu.y + h > vh - 4.0 {
            (vh - h - 4.0).max(4.0)
        } else {
            menu.y
        };
        pos.set((x, y));
    });
    let items = match menu.target {
        CtxTarget::Message { msg, mine, quote } => message_menu_items(c, *msg, mine, quote).into_any(),
        CtxTarget::Chat(id) => chat_menu_items(c, id).into_any(),
        CtxTarget::SidebarPrefs => sidebar_menu_items(c).into_any(),
        CtxTarget::User(id) => user_menu_items(c, id).into_any(),
        CtxTarget::Section(id) => section_menu_items(c, id).into_any(),
        CtxTarget::Workspace => workspace_menu_items(c).into_any(),
        CtxTarget::AddChannels => view! {
            <button on:click=move |_| { close_menus(c); c.creating_channel.set(Some(NewChannel::default())); }>{tr().create_channel}</button>
            <button on:click=move |_| { close_menus(c); c.browsing.set(true); }>{tr().browse_channels}</button>
        }
        .into_any(),
        CtxTarget::Organizations => {
            view! { <div class="menu-label">{tr().organizations}</div>{organization_menu_items(c)} }.into_any()
        }
    };
    view! {
        <div class="popover menu ctx-menu" node_ref=node
            style=move || { let (x, y) = pos.get(); format!("left:{x}px;top:{y}px") }
            on:contextmenu=|e| e.prevent_default()>
            {items}
        </div>
    }
}

/// Person menu, like the one on an author's name in Slack: profile, direct message, copy name.
fn user_menu_items(c: Ctx, id: UserId) -> impl IntoView {
    let name = c
        .snap
        .with_untracked(|s| s.user(&id).map(|u| u.display_name.clone()))
        .unwrap_or_default();
    let me = c.snap.with_untracked(|s| s.me == id);
    let (p_id, m_id) = (id.clone(), id);
    view! {
        <button on:click=move |_| { close_menus(c); ops::open_profile(c, p_id.clone()); }>{tr().view_full_profile}</button>
        {(!me).then(|| view! {
            <button on:click=move |_| { close_menus(c); ops::message_user(c, m_id.clone()); }>{tr().message_button}</button>
        })}
        <button on:click=move |_| { close_menus(c); let _ = window().navigator().clipboard().write_text(&name); }>{tr().copy_name}</button>
    }
}

/// Section menu, as in Slack: the "⋮" of a section header or a right click on it. What the
/// section makes (channels, messages), managing it, the sidebar filter and sorting, deleting it.
fn section_menu_items(c: Ctx, id: String) -> impl IntoView {
    let unread = c
        .sections
        .with_untracked(|list| {
            list.iter().find(|sec| sec.id == id).map(|sec| {
                c.snap.with_untracked(|s| {
                    sec.chats
                        .iter()
                        .filter(|ch| s.chat(ch).is_some_and(|ch| ch.unread > 0))
                        .cloned()
                        .collect::<Vec<_>>()
                })
            })
        })
        .unwrap_or_default();
    let title = c
        .sections
        .with_untracked(|list| list.iter().find(|sec| sec.id == id).map(|sec| sec.title.clone()))
        .unwrap_or_default();
    let all: Vec<String> = c
        .sections
        .with_untracked(|list| list.iter().map(|sec| sec.id.clone()).collect());
    let collapsed = c.collapsed.with_untracked(|set| set.contains(&id));
    let toggle_id = id.clone();
    let custom = c.settings.with_untracked(|s| s.sidebar_sort == SidebarSort::Custom)
        && c.chat_order.with_untracked(|o| o.contains_key(&id));
    let folder = folders::is_folder(&id);
    // What the section makes: channels in "Channels" and one's own sections, messages in "Direct messages".
    let make = match id.as_str() {
        "channels" => Some(view! {
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); c.creating_channel.set(Some(NewChannel::default())); }>
                <Icon icon=I::Hash size=16/><span>{tr().create_channel}</span>
            </button>
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); c.browsing.set(true); }>
                <Icon icon=I::Search size=16/><span>{tr().browse_channels}</span>
            </button>
        }
        .into_any()),
        "dms" => Some(view! {
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); c.new_message.set(true); }>
                <Icon icon=I::Compose size=16/><span>{tr().new_message}</span>
            </button>
        }
        .into_any()),
        _ if folder => {
            let to = id.clone();
            Some(view! {
                <button class="menu-icon-item" on:click=move |_| {
                    close_menus(c);
                    c.creating_channel.set(Some(NewChannel { folder: Some(to.clone()) }));
                }>
                    <Icon icon=I::Hash size=16/><span>{tr().create_channel}</span>
                </button>
            }
            .into_any())
        }
        _ => None,
    };
    // An own section can be renamed and deleted: its chats go back to their sections.
    let rename = folder.then(|| {
        let rename_id = id.clone();
        view! {
            <button on:click=move |_| { close_menus(c); c.folder_edit.set(Some(FolderEdit::Rename(rename_id.clone()))); }>{tr().rename_section}</button>
        }
    });
    let delete = folder.then(|| {
        let delete_id = id.clone();
        view! {
            <div class="menu-sep"></div>
            <button class="danger" on:click=move |_| {
                close_menus(c);
                ops::change_folders(c, |f| f.retain(|f| f.id != delete_id));
                c.collapsed.update(|set| { set.remove(&delete_id); });
            }>{tr().delete_section}</button>
        }
    });
    let reset = custom.then(|| {
        let id = id.clone();
        view! {
            <button on:click=move |_| { close_menus(c); c.chat_order.update(|o| { o.remove(&id); }); }>{tr().reset_order}</button>
        }
    });
    view! {
        <div class="menu-label">{tr().section(&id, &title)}</div>
        {make}
        <button disabled=unread.is_empty() on:click=move |_| {
            close_menus(c);
            for ch in unread.clone() {
                ops::mark_chat_read(c, ch);
            }
        }>{tr().mark_all_read}</button>
        <div class="menu-sep"></div>
        <div class="menu-label">{tr().manage_section}</div>
        {rename}
        <button on:click=move |_| { close_menus(c); c.folder_edit.set(Some(FolderEdit::New(None))); }>{tr().create_section}</button>
        <button on:click=move |_| {
            close_menus(c);
            c.collapsed.update(|set| if !set.remove(&toggle_id) { set.insert(toggle_id.clone()); });
        }>{if collapsed { tr().expand_section } else { tr().collapse_section }}</button>
        <button on:click=move |_| { close_menus(c); c.collapsed.set(all.iter().cloned().collect()); }>{tr().collapse_all}</button>
        <button on:click=move |_| { close_menus(c); c.collapsed.set(HashSet::new()); }>{tr().expand_all}</button>
        {reset}
        <div class="menu-sep"></div>
        // Filter and sorting are common to all sections in Mostik, so the label says so.
        <div class="menu-label">{tr().sidebar_settings}</div>
        {filter_submenu(c)}
        {sort_submenu(c)}
        {delete}
    }
}

/// A submenu row: the name, the current value on the right and the choices on hover.
fn choice_submenu(
    label: &'static str,
    value: Signal<&'static str>,
    items: impl Fn() -> AnyView + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <div class="me-sub" on:mouseenter=move |_| open.set(true) on:mouseleave=move |_| open.set(false)>
            <button class="me-sub-btn" on:click=move |_| open.set(true)>
                <span>{label}</span>
                <span class="menu-value">{value}<Icon icon=I::ChevronRight size=14/></span>
            </button>
            <Show when=move || open.get()>
                <div class="me-submenu">{items()}</div>
            </Show>
        </div>
    }
}

/// A radio item of a submenu: a tick at the current value.
fn radio_item(
    c: Ctx,
    label: &'static str,
    on: impl Fn(&Settings) -> bool + Copy + Send + Sync + 'static,
    set: impl Fn(&mut Settings) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <button class="menu-radio" on:click=move |_| { close_menus(c); c.settings.update(set); }>
            <span class="menu-tick">{move || c.settings.with(on).then(|| view! { <Icon icon=I::Check size=14/> })}</span>
            <span>{label}</span>
        </button>
    }
}

fn filter_submenu(c: Ctx) -> impl IntoView {
    let label = |f: SidebarFilter| match f {
        SidebarFilter::Active => tr().f_active,
        SidebarFilter::Unreads => tr().f_unreads,
        SidebarFilter::Mentions => tr().f_mentions,
        SidebarFilter::All => tr().f_all,
    };
    let item = move |f: SidebarFilter| {
        radio_item(
            c,
            label(f),
            move |s| s.sidebar_filter == f,
            move |s| s.sidebar_filter = f,
        )
    };
    let items = move || {
        view! {
            {item(SidebarFilter::Active)}
            {item(SidebarFilter::Unreads)}
            {item(SidebarFilter::Mentions)}
            {item(SidebarFilter::All)}
        }
        .into_any()
    };
    choice_submenu(
        tr().filter,
        Signal::derive(move || label(c.settings.with(|s| s.sidebar_filter))),
        items,
    )
}

fn sort_submenu(c: Ctx) -> impl IntoView {
    let label = |o: SidebarSort| match o {
        SidebarSort::Alphabetical => tr().s_az,
        SidebarSort::Recency => tr().s_recency,
        SidebarSort::Custom => tr().s_custom,
    };
    let item = move |o: SidebarSort| radio_item(c, label(o), move |s| s.sidebar_sort == o, move |s| s.sidebar_sort = o);
    let items = move || {
        view! {
            {item(SidebarSort::Alphabetical)}
            {item(SidebarSort::Recency)}
            {item(SidebarSort::Custom)}
        }
        .into_any()
    };
    choice_submenu(
        tr().sort,
        Signal::derive(move || label(c.settings.with(|s| s.sidebar_sort))),
        items,
    )
}

const ORG_KEY: &str = "mostik.org";

/// The key of the current organization in Telemost settings such as chat folders: its id, "0"
/// without organizations.
pub fn org_key(s: &Snapshot, chosen: Option<u64>) -> String {
    s.organizations
        .iter()
        .find(|o| Some(o.id) == chosen)
        .or(s.organizations.first())
        .map_or_else(|| "0".to_owned(), |o| o.id.to_string())
}

/// Current organization: the selected one, or the first if none is selected.
pub(crate) fn current_org(c: Ctx) -> Option<Organization> {
    let chosen = c.org.get();
    c.snap.with(|s| {
        s.organizations
            .iter()
            .find(|o| Some(o.id) == chosen)
            .or(s.organizations.first())
            .cloned()
    })
}

fn org_name(c: Ctx) -> String {
    current_org(c)
        .map(|o| o.name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| c.snap.with(|s| s.organization.clone()))
}

fn org_avatar(c: Ctx) -> Option<String> {
    current_org(c)
        .and_then(|o| o.avatar)
        .or_else(|| c.snap.with(|s| s.organization_avatar.clone()))
}

/// The chat is visible in the selected organization: direct chats always, the rest if they belong to it.
/// The filter applies only when there is more than one organization.
fn in_current_org(c: Ctx, ch: &Chat) -> bool {
    let many = c.snap.with(|s| s.organizations.len() > 1);
    if !many || ch.organizations.is_empty() {
        return true;
    }
    current_org(c).is_none_or(|o| ch.organizations.contains(&o.id))
}

/// Opens a menu at the bottom left corner of the clicked element.
fn open_menu_below(c: Ctx, e: &ev::MouseEvent, target: CtxTarget) {
    let Some(el) = e
        .current_target()
        .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
    else {
        return;
    };
    let r = el.get_bounding_client_rect();
    c.ctx_menu.set(Some(CtxMenu {
        x: r.left(),
        y: r.bottom() + 4.0,
        target,
    }));
}

/// Organization menu, like the workspace menu in Slack: name, preferences, sign out.
fn workspace_menu_items(c: Ctx) -> impl IntoView {
    let name = org_name(c);
    let face = org_avatar(c);
    let many = c.snap.with_untracked(|s| s.organizations.len() > 1);
    view! {
        <div class="menu-head ws-head">
            <span class="ws-tile small">
                {name.chars().next().unwrap_or('?').to_string()}
                {face.map(|id| view! { <img class="ws-img" src=avatar_url(&id, 72) alt=""/> })}
            </span>
            <b>{name.clone()}</b>
        </div>
        <div class="menu-sep"></div>
        <button on:click=move |_| { close_menus(c); c.prefs.set(true); }>{tr().preferences}</button>
        <button on:click=move |_| { close_menus(c); c.creating_channel.set(Some(NewChannel::default())); }>{tr().create_channel}</button>
        <button on:click=move |_| { close_menus(c); c.browsing.set(true); }>{tr().browse_channels}</button>
        {many.then(|| view! {
            <div class="menu-sep"></div>
            <div class="menu-label">{tr().switch_org}</div>
            {organization_menu_items(c)}
        })}
        {c.live.then(|| view! {
            <div class="menu-sep"></div>
            <button on:click=move |_| {
                close_menus(c);
                ops::sign_out(c);
            }>{tr().sign_out_of(&name)}</button>
        })}
    }
}

/// List of organizations with a check mark at the current one; picking one switches the sidebar.
fn organization_menu_items(c: Ctx) -> impl IntoView {
    let current = current_org(c).map(|o| o.id);
    let list = c.snap.with_untracked(|s| s.organizations.clone());
    if list.is_empty() {
        let name = org_name(c);
        return view! { <button class="menu-radio checked" disabled=true><span class="menu-tick">"✓"</span>{name}</button> }.into_any();
    }
    list.into_iter()
        .map(|o| {
            let checked = Some(o.id) == current;
            let id = o.id;
            view! {
                <button class=if checked { "menu-radio checked" } else { "menu-radio" } on:click=move |_| {
                    close_menus(c);
                    c.org.set(Some(id));
                    if let Some(st) = window().local_storage().ok().flatten() {
                        let _ = st.set_item(ORG_KEY, &id.to_string());
                    }
                }>
                    <span class="menu-tick">{if checked { "✓" } else { "" }}</span>{o.name}
                </button>
            }
        })
        .collect_view()
        .into_any()
}

fn sidebar_menu_items(c: Ctx) -> impl IntoView {
    let sub_open = RwSignal::new(false);
    let pick = move |f: fn(&mut Settings)| {
        close_menus(c);
        c.settings.update(f);
    };
    let filter_item = move |value: SidebarFilter, label: &'static str| {
        let on = move || c.settings.with(|s| s.sidebar_filter == value);
        view! {
            <button class="menu-radio" on:click=move |_| { close_menus(c); c.settings.update(|s| s.sidebar_filter = value); }>
                <span class="menu-tick">{move || on().then(|| view! { <Icon icon=I::Check size=14/> })}</span>
                <span>{label}</span>
            </button>
        }
    };
    let sort_item = move |value: SidebarSort, label: &'static str| {
        let on = move || c.settings.with(|s| s.sidebar_sort == value);
        view! {
            <button class="menu-radio" on:click=move |_| { close_menus(c); c.settings.update(|s| s.sidebar_sort = value); }>
                <span class="menu-tick">{move || on().then(|| view! { <Icon icon=I::Check size=14/> })}</span>
                <span>{label}</span>
            </button>
        }
    };
    view! {
        <div class="me-sub" on:mouseenter=move |_| sub_open.set(true) on:mouseleave=move |_| sub_open.set(false)>
            <button class="me-sub-btn" on:click=move |_| sub_open.set(true)>
                <span>{tr().filter_and_sort}</span><Icon icon=I::ChevronRight size=14/>
            </button>
            <Show when=move || sub_open.get()>
                <div class="me-submenu">
                    <div class="menu-label">{tr().filter_by}</div>
                    {filter_item(SidebarFilter::Active, tr().f_active)}
                    {filter_item(SidebarFilter::Unreads, tr().f_unreads)}
                    {filter_item(SidebarFilter::Mentions, tr().f_mentions)}
                    {filter_item(SidebarFilter::All, tr().f_all)}
                    <div class="menu-sep"></div>
                    <div class="menu-label">{tr().sort_by}</div>
                    {sort_item(SidebarSort::Alphabetical, tr().s_az)}
                    {sort_item(SidebarSort::Recency, tr().s_recency)}
                    {sort_item(SidebarSort::Custom, tr().s_custom)}
                </div>
            </Show>
        </div>
        <div class="menu-sep"></div>
        {move || if c.settings.with(|s| s.hide_muted) {
            view! { <button on:click=move |_| pick(|s| s.hide_muted = false)>{tr().show_muted}</button> }.into_any()
        } else {
            view! { <button on:click=move |_| pick(|s| s.hide_muted = true)>{tr().hide_muted}</button> }.into_any()
        }}
        <button on:click=move |_| { close_menus(c); c.folder_edit.set(Some(FolderEdit::New(None))); }>{tr().create_section}</button>
    }
}

/// Sidebar row menu, as in Slack: channel details or the peer's profile, copying,
/// "Mark as read", starring, notifications and leaving the channel.
fn chat_menu_items(c: Ctx, id: ChatId) -> impl IntoView {
    let Some((kind, title, me, unread, notify)) = c.snap.with_untracked(|s| {
        s.chat(&id).map(|ch| {
            (
                ch.kind.clone(),
                display_title(s, ch),
                s.me.clone(),
                ch.unread > 0,
                ch.notify_level(),
            )
        })
    }) else {
        return ().into_any();
    };
    let starred = ops::is_starred(c, &id);
    let (leave_id, star_id, mute_id, read_id, open_id, hide_id) =
        (id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    let notes = kind == ChatKind::Notes;
    let move_to = (!notes).then(|| move_to_section_items(c, id.clone()));
    let copy = move |_| {
        close_menus(c);
        let _ = window().navigator().clipboard().write_text(&title);
    };
    let head = match kind.clone() {
        ChatKind::Group { .. } | ChatKind::Broadcast => view! {
            <button on:click=move |_| {
                close_menus(c);
                ops::open_chat(c, open_id.clone());
                c.details.set(Some(DetailsTab::About));
            }>{tr().view_channel_details}</button>
        }
        .into_any(),
        ChatKind::Direct { peer } => view! {
            <button on:click=move |_| { close_menus(c); ops::open_profile(c, peer.clone()); }>{tr().view_full_profile}</button>
        }
        .into_any(),
        ChatKind::Notes => view! {
            <button on:click=move |_| { close_menus(c); ops::open_profile(c, me.clone()); }>{tr().view_full_profile}</button>
        }
        .into_any(),
    };
    let channel = matches!(kind, ChatKind::Group { .. } | ChatKind::Broadcast);
    view! {
        {head}
        <button on:click=copy>{tr().copy_name}</button>
        <div class="menu-sep"></div>
        {unread.then(|| view! {
            <button on:click=move |_| { close_menus(c); ops::mark_chat_read(c, read_id.clone()); }>{tr().mark_as_read}</button>
        })}
        <button on:click=move |_| { close_menus(c); ops::toggle_star(c, star_id.clone()); }>
            {if starred { tr().unstar } else { tr().star }}
        </button>
        {move_to}
        {(!notes).then(|| view! {
            <button on:click=move |_| { close_menus(c); ops::hide_chat(c, hide_id.clone()); }>{tr().hide_chat}</button>
        })}
        <div class="menu-sep"></div>
        <div class="menu-label">{tr().notify_label}</div>
        {[NotifyLevel::All, NotifyLevel::Mentions, NotifyLevel::Nothing].map(|level| {
            let id = mute_id.clone();
            view! {
                <button class="menu-radio" on:click=move |_| { close_menus(c); ops::set_notify_level(c, id.clone(), level); }>
                    <span class="menu-tick">{(level == notify).then(|| view! { <Icon icon=I::Check size=14/> })}</span>
                    <span>{tr().notify_level(level)}</span>
                </button>
            }
        }).collect_view()}
        {channel.then(|| view! {
            <div class="menu-sep"></div>
            <button class="danger" on:click=move |_| {
                close_menus(c);
                c.confirm_leave.set(Some(leave_id.clone()));
            }>{tr().leave_channel}</button>
        })}
    }
    .into_any()
}

/// "Move to section" submenu of a chat: its own Telemost section, the folders and a new one.
fn move_to_section_items(c: Ctx, id: ChatId) -> impl IntoView {
    let sub_open = RwSignal::new(false);
    let apart = c.settings.with_untracked(|s| s.group_calls_apart);
    let home = c
        .snap
        .with_untracked(|s| s.chat(&id).map(|ch| folders::shown_section(ch, apart)))
        .unwrap_or("channels");
    let (list, current) = c.folders.with_untracked(|f| {
        (
            f.iter().map(|f| (f.id.clone(), folders::label(f))).collect::<Vec<_>>(),
            folders::folder_of(f, &id).map(|f| f.id.clone()),
        )
    });
    let item = {
        let id = id.clone();
        move |to: Option<String>, label: String| {
            let on = to == current;
            let id = id.clone();
            view! {
                <button class="menu-radio" on:click=move |_| {
                    close_menus(c);
                    ops::change_folders(c, |f| folders::move_chat(f, &id, to.as_deref(), None));
                }>
                    <span class="menu-tick">{on.then(|| view! { <Icon icon=I::Check size=14/> })}</span>
                    <span>{label}</span>
                </button>
            }
        }
    };
    let targets: Vec<(Option<String>, String)> = std::iter::once((None, tr().section(home, "")))
        .chain(list.into_iter().map(|(fid, name)| (Some(fid), name)))
        .collect();
    let items = move || {
        targets
            .iter()
            .map(|(to, label)| item(to.clone(), label.clone()))
            .collect_view()
    };
    view! {
        <div class="me-sub" on:mouseenter=move |_| sub_open.set(true) on:mouseleave=move |_| sub_open.set(false)>
            <button class="me-sub-btn" on:click=move |_| sub_open.set(true)>
                <span>{tr().move_to_section}</span><Icon icon=I::ChevronRight size=14/>
            </button>
            <Show when=move || sub_open.get()>
                <div class="me-submenu">
                    {items.clone()}
                    <div class="menu-sep"></div>
                    <button on:click={
                        let id = id.clone();
                        move |_| { close_menus(c); c.folder_edit.set(Some(FolderEdit::New(Some(id.clone())))); }
                    }>{tr().new_section_item}</button>
                </div>
            </Show>
        </div>
    }
}

/// Search result row.
#[derive(Clone)]
enum Hit {
    Chat(ChatId, String, Mark),
    Person(User),
    Message(Box<Message>, String),
}

#[component]
fn TopBar() -> impl IntoView {
    let c = ctx();
    let input = NodeRef::<html::Input>::new();
    let open = RwSignal::new(false);
    let history_open = RwSignal::new(false);
    let selected = RwSignal::new(0usize);
    let hits: RwSignal<Vec<Hit>> = RwSignal::new(vec![]);
    let placeholder = move || tr().search_in(&org_name(c));
    // Ctrl+K is the quick switcher, Alt+← / Alt+→ go back and forward, as in Slack.
    scoped_window_listener(ev::keydown, move |e| {
        if e.ctrl_key() && e.key().eq_ignore_ascii_case("k") {
            e.prevent_default();
            if let Some(el) = input.get_untracked() {
                let _ = el.focus();
                open.set(true);
            }
        } else if e.key() == "Escape" && history_open.get_untracked() {
            history_open.set(false);
        } else if e.alt_key() && !e.ctrl_key() && (e.key() == "ArrowLeft" || e.key() == "ArrowRight") {
            e.prevent_default();
            ops::navigate(c, e.key() == "ArrowLeft");
        }
    });
    // A click outside the search and history closes their drop-downs.
    scoped_window_listener(ev::mousedown, move |e| {
        let inside = |sel: &str| {
            e.target()
                .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
                .and_then(|el| el.closest(sel).ok().flatten())
                .is_some()
        };
        if !inside(".search-wrap") {
            open.set(false);
        }
        if !inside(".history-wrap") {
            history_open.set(false);
        }
    });
    let activate = move |hit: Hit| {
        open.set(false);
        c.search.set(String::new());
        if let Some(el) = input.get_untracked() {
            let _ = el.blur();
        }
        match hit {
            Hit::Chat(id, _, _) => ops::open_chat(c, id),
            Hit::Person(u) => ops::open_profile(c, u.id),
            Hit::Message(m, _) => ops::jump_to_message(c, m.chat_id.clone(), m.ts, m.thread_parent),
        }
    };
    let on_key = move |e: ev::KeyboardEvent| {
        let n = hits.with_untracked(Vec::len);
        match e.key().as_str() {
            "Escape" => {
                e.stop_propagation();
                if c.search.with_untracked(String::is_empty) {
                    open.set(false);
                    if let Some(el) = input.get_untracked() {
                        let _ = el.blur();
                    }
                } else {
                    c.search.set(String::new());
                }
            }
            "ArrowDown" if n > 0 => {
                e.prevent_default();
                selected.update(|i| *i = (*i + 1) % n);
            }
            "ArrowUp" if n > 0 => {
                e.prevent_default();
                selected.update(|i| *i = (*i + n - 1) % n);
            }
            "Enter" => {
                e.prevent_default();
                if let Some(hit) = hits.with_untracked(|h| h.get(selected.get_untracked()).cloned()) {
                    activate(hit);
                }
            }
            _ => {}
        }
    };
    let can_back = move || c.nav_back.with(|v| !v.is_empty());
    let can_forward = move || c.nav_forward.with(|v| !v.is_empty());
    view! {
        <header class="topbar">
            <div class="topbar-nav">
                <button class="tb-btn" title=tr().back disabled=move || !can_back() on:click=move |_| ops::navigate(c, true)><Icon icon=I::ArrowLeft/></button>
                <button class="tb-btn" title=tr().forward disabled=move || !can_forward() on:click=move |_| ops::navigate(c, false)><Icon icon=I::ArrowRight/></button>
                <div class="history-wrap">
                    <button class=move || if history_open.get() { "tb-btn active" } else { "tb-btn" } title=tr().history
                        on:click=move |_| history_open.update(|v| *v = !*v)><Icon icon=I::Clock/></button>
                    <Show when=move || history_open.get()>
                        <HistoryMenu on_close=Callback::new(move |_| history_open.set(false))/>
                    </Show>
                </div>
            </div>
            <div class="search-wrap">
                <label class="search">
                    <Icon icon=I::Search size=15/>
                    <input
                        node_ref=input
                        placeholder=placeholder
                        prop:value=move || c.search.get()
                        on:focus=move |_| open.set(true)
                        on:input=move |e| { c.search.set(event_target_value(&e)); selected.set(0); open.set(true); }
                        on:keydown=on_key
                    />
                    <span class="kbd">"Ctrl K"</span>
                </label>
                <Show when=move || open.get()>
                    <SearchResults hits=hits selected=selected on_pick=Callback::new(activate)/>
                </Show>
            </div>
            <div class="topbar-right">
                <UpdateButton/>
                <button class="tb-btn" title=tr().about on:click=move |_| c.about.set(true)>
                    <Icon icon=I::Help/>
                </button>
            </div>
        </header>
    }
}

/// "Update" in the top bar, as in Slack: shown when a new release is out; a click installs it
/// and restarts the app.
#[component]
fn UpdateButton() -> impl IntoView {
    let c = ctx();
    move || {
        match c.update.get() {
        _ if !c.settings.with(|s| s.auto_update) => None,
        bridge::UpdateStatus::None | bridge::UpdateStatus::Unsupported => None,
        bridge::UpdateStatus::Available { version } => Some(
            view! {
                <button class="update-btn" title=tr().update_available(&version) on:click=move |_| ops::install_update(c)>
                    <Icon icon=I::Download size=15/>
                    {tr().update_button}
                </button>
            }
            .into_any(),
        ),
        bridge::UpdateStatus::Installing { version, percent } => Some(
            view! {
                <button class="update-btn" disabled=true title=tr().update_available(&version)>
                    <span class="update-spinner"></span>
                    {tr().updating(percent)}
                </button>
            }
            .into_any(),
        ),
    }
    }
}

/// "History" in the top bar: recently opened chats.
#[component]
fn HistoryMenu(on_close: Callback<()>) -> impl IntoView {
    let c = ctx();
    let rows = move || {
        c.snap.with(|s| {
            c.recent
                .get()
                .into_iter()
                .filter_map(|id| {
                    s.chat(&id)
                        .map(|ch| (id, display_title(s, ch), Mark::of(ch), chat_face(s, ch)))
                })
                .collect::<Vec<_>>()
        })
    };
    view! {
        <div class="popover menu history-menu">
            <div class="sr-title">{tr().recent_chats}</div>
            {move || {
                let list = rows();
                if list.is_empty() {
                    return view! { <div class="sr-empty">{tr().no_history}</div> }.into_any();
                }
                list.into_iter().map(|(id, title, kind, face)| view! {
                    <button class="sr-item" on:click=move |_| { on_close.run(()); ops::open_chat(c, id.clone()); }>
                        <span class="sr-icon">{match face {
                            Some(u) => view! { <Avatar user=u size=20/> }.into_any(),
                            None => kind_icon(&kind, 15),
                        }}</span>
                        <span>{title}</span>
                    </button>
                }).collect_view().into_any()
            }}
        </div>
    }
}

#[component]
fn SearchResults(hits: RwSignal<Vec<Hit>>, selected: RwSignal<usize>, on_pick: Callback<Hit>) -> impl IntoView {
    let c = ctx();
    // Server results: messages from all chats and people from the directory (app only).
    let remote: RwSignal<(String, Vec<Message>, Vec<User>)> = RwSignal::new((String::new(), vec![], vec![]));
    let loading = RwSignal::new(false);
    let seq = RwSignal::new(0u32);
    Effect::new(move |_| {
        let q = c.search.get().trim().to_owned();
        if !c.live || q.chars().count() < 2 {
            loading.set(false);
            return;
        }
        let n = seq.get_untracked() + 1;
        seq.set(n);
        loading.set(true);
        // The search may close before the timer fires or the answer comes: its signals are gone then.
        set_timeout(
            move || {
                if seq.try_get_untracked() != Some(n) {
                    return;
                }
                spawn_local(async move {
                    // The server searches within one chat: the current one and the most recent ones.
                    let chats: Vec<String> = c.snap.with_untracked(|s| {
                        let cur = c.current.get_untracked();
                        let mut list: Vec<&Chat> = s.chats.iter().filter(|ch| ch.id != cur).collect();
                        list.sort_by_key(|ch| std::cmp::Reverse(ch.last_ts));
                        std::iter::once(cur.0)
                            .chain(list.into_iter().map(|ch| ch.id.0.clone()))
                            .filter(|id| !id.is_empty())
                            .take(20)
                            .collect()
                    });
                    let found = bridge::search_messages(&q, chats).await;
                    let people = bridge::search_people(&q, None).await;
                    if seq.try_get_untracked() != Some(n) {
                        return;
                    }
                    let mut users = found.as_ref().map(|f| f.users.clone()).unwrap_or_default();
                    let people: Vec<User> = people.unwrap_or_default().iter().map(profile_face).collect();
                    c.snap.update(|s| {
                        for u in users.drain(..).chain(people.iter().cloned()) {
                            if s.user(&u.id).is_none() {
                                s.users.push(u);
                            }
                        }
                    });
                    if let Err(e) = &found {
                        leptos::logging::warn!("search: {e}");
                    }
                    remote.try_set((q, found.map(|f| f.messages).unwrap_or_default(), people));
                    loading.try_set(false);
                });
            },
            Duration::from_millis(300),
        );
    });
    Effect::new(move |_| {
        let q = c.search.get().trim().to_lowercase();
        if q.is_empty() {
            return hits.set(vec![]);
        }
        let list = c.snap.with(|s| {
            let mut out: Vec<Hit> = s
                .chats
                .iter()
                .filter(|ch| !matches!(ch.kind, ChatKind::Direct { .. }))
                .map(|ch| (ch.id.clone(), display_title(s, ch), Mark::of(ch)))
                .filter(|(_, t, _)| t.to_lowercase().contains(&q))
                .take(6)
                .map(|(id, t, k)| Hit::Chat(id, t, k))
                .collect();
            let mut people: Vec<User> = s
                .users
                .iter()
                .filter(|u| !u.is_robot && u.display_name.to_lowercase().contains(&q))
                .cloned()
                .collect();
            remote.with(|(rq, _, rp)| {
                if rq.to_lowercase() == q {
                    for u in rp {
                        if !people.iter().any(|p| p.id == u.id) {
                            people.push(u.clone());
                        }
                    }
                }
            });
            out.extend(people.into_iter().take(6).map(Hit::Person));
            let mut messages: Vec<Message> = s.mention_names().with(|who| {
                s.messages
                    .iter()
                    .filter(|m| mostik_core::markup::plain(&m.text, who).to_lowercase().contains(&q))
                    .cloned()
                    .collect()
            });
            remote.with(|(rq, rm, _)| {
                if rq.to_lowercase() == q {
                    for m in rm {
                        if !messages.iter().any(|x| x.chat_id == m.chat_id && x.ts == m.ts) {
                            messages.push(m.clone());
                        }
                    }
                }
            });
            messages.sort_by_key(|m| std::cmp::Reverse(m.ts));
            out.extend(messages.into_iter().take(10).map(|m| {
                let chat = s.chat(&m.chat_id).map(|ch| display_title(s, ch)).unwrap_or_default();
                Hit::Message(Box::new(m), chat)
            }));
            out
        });
        if selected.get_untracked() >= list.len() {
            selected.set(0);
        }
        hits.set(list);
    });
    move || {
        let q = c.search.get();
        if q.trim().is_empty() {
            return None;
        }
        let list = hits.get();
        let sel = selected.get();
        let mut last_kind = "";
        let rows = list
            .into_iter()
            .enumerate()
            .map(|(i, hit)| {
                let kind = match &hit { Hit::Chat(..) => "chat", Hit::Person(_) => "person", Hit::Message(..) => "msg" };
                let title = (kind != last_kind).then(|| {
                    let t = match kind { "chat" => tr().search_chats, "person" => tr().search_people, _ => tr().search_messages };
                    view! { <div class="sr-title">{t}</div> }
                });
                last_kind = kind;
                let cls = if i == sel { "sr-item selected" } else { "sr-item" };
                let picked = hit.clone();
                let body = match hit {
                    Hit::Chat(_, title, kind) => view! {
                        <span class="sr-icon">{kind_icon(&kind, 15)}</span><span>{title}</span>
                    }.into_any(),
                    Hit::Person(u) => {
                        let name = u.display_name.clone();
                        let title = u.title.clone().unwrap_or_default();
                        view! {
                            <span class="sr-icon"><Avatar user=u size=20 presence=true/></span>
                            <span>{name}</span><span class="sr-sub">{title}</span>
                        }.into_any()
                    }
                    Hit::Message(m, chat) => {
                        let author = c.snap.with_untracked(|s| s.user(&m.author).map(|u| u.display_name.clone())).unwrap_or_default();
                        let text: String = c.snap.with(|s| s.plain(&m.text)).chars().take(120).collect();
                        view! {
                            <span class="sr-msg-head"><span class="sr-chat">{chat}</span><span class="sr-sub">{author}" · "{time::relative(m.ts)}</span></span>
                            <span class="sr-text">{text}</span>
                        }.into_any()
                    }
                };
                let msg_cls = if kind == "msg" { format!("{cls} msg") } else { cls.to_owned() };
                view! {
                    {title}
                    <button class=msg_cls on:mouseenter=move |_| selected.set(i) on:click=move |_| on_pick.run(picked.clone())>{body}</button>
                }
            })
            .collect_view();
        let empty = hits.with(Vec::is_empty);
        Some(view! {
            <div class="search-results">
                {rows}
                {move || loading.get().then(|| view! { <div class="sr-empty">{tr().searching}</div> })}
                {move || (empty && !loading.get()).then(|| view! { <div class="sr-empty">{tr().nothing_found}</div> })}
            </div>
        })
    }
}

#[component]
fn Rail() -> impl IntoView {
    let c = ctx();
    let org = move || org_name(c);
    let initial = move || org_name(c).chars().next().unwrap_or('?').to_string();
    let me = move || c.snap.with(|s| s.user(&s.me).cloned());
    let item = move |icon: I, label: &'static str, mode: ViewMode| {
        view! {
            <button class=move || if c.view.get() == mode || (mode == ViewMode::Home && c.view.get().is_home_page()) { "rail-item active" } else { "rail-item" }
                on:click=move |_| c.view.set(mode)>
                <span class="rail-icon"><Icon icon=icon size=20/>
                    {(mode == ViewMode::Activity).then_some(move || {
                        let n: u32 = c.snap.with(|s| s.chats.iter().map(|ch| ch.mentions).sum());
                        (n > 0).then(|| view! { <span class="rail-badge">{n}</span> })
                    })}
                </span>
                <span class="rail-label">{label}</span>
            </button>
        }
    };
    view! {
        <nav class="rail">
            // As in Slack: the organization tile opens the organization switcher.
            <button class="ws-tile" title=org
                on:mousedown=|e| e.stop_propagation()
                on:click=move |e| {
                    if c.ctx_menu.with_untracked(|m| m.as_ref().is_some_and(|m| matches!(m.target, CtxTarget::Organizations))) {
                        return c.ctx_menu.set(None);
                    }
                    let Some(el) = e.current_target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()) else { return };
                    let r = el.get_bounding_client_rect();
                    c.ctx_menu.set(Some(CtxMenu { x: r.right() + 8.0, y: r.top(), target: CtxTarget::Organizations }));
                }>
                {initial}
                {move || org_avatar(c).map(|id| {
                    let failed = RwSignal::new(false);
                    view! { <Show when=move || !failed.get()><img class="ws-img" src=avatar_url(&id, 72) alt="" on:error=move |_| failed.set(true)/></Show> }
                })}
            </button>
            {item(I::Home, tr().rail_home, ViewMode::Home)}
            {item(I::Chat, tr().rail_dms, ViewMode::Dms)}
            {item(I::Activity, tr().rail_activity, ViewMode::Activity)}
            <div class="rail-spacer"></div>
            // As in Slack: with a status set, an emoji badge sits above the avatar and together they form one "capsule".
            <button class=move || if me().is_some_and(|u| u.status.is_some()) { "rail-me with-status" } else { "rail-me" }
                title=tr().profile_and_prefs on:click=move |_| c.me_menu.update(|v| *v = !*v)>
                {move || me().and_then(|u| u.status).map(|st| {
                    view! { <span class="rail-status" data-tip=status_tip(&st)>{status_icon(&st, 18)}</span> }
                })}
                {move || me().map(|u| view! { <Avatar user=u size=36 presence=true/> })}
            </button>
            <Show when=move || c.me_menu.get()>
                <MeMenu/>
            </Show>
        </nav>
    }
}

const AVATAR_COLORS: [&str; 8] = [
    "#E8912D", "#2BAC76", "#1264A3", "#9E3997", "#CD2553", "#3C989F", "#7C3085", "#E06B56",
];

fn avatar_color(id: &str) -> &'static str {
    let hash = id.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    AVATAR_COLORS[(hash % AVATAR_COLORS.len() as u32) as usize]
}

fn initials(name: &str, size: u32) -> String {
    let letters = if size < 24 { 1 } else { 2 };
    name.split_whitespace()
        .take(letters)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

/// Presence dot, as in Slack: green is online, hollow is away or offline,
/// with a "z" is do not disturb, red is busy.
fn presence_class(u: &User) -> &'static str {
    match (u.availability, u.presence) {
        (Availability::DoNotDisturb, _) => "presence dnd",
        (Availability::Busy, _) => "presence busy",
        (Availability::Away, _) => "presence away",
        (_, Presence::Online) => "presence online",
        (_, Presence::Away) => "presence away",
        (_, Presence::Offline) => "presence offline",
    }
}

/// Status emoji after the name, with the status text in a hover tip.
fn status_badge(u: &User) -> Option<AnyView> {
    let st = u.status.clone()?;
    Some(view! { <span class="status-emoji" data-tip=status_tip(&st)>{status_icon(&st, 14)}</span> }.into_any())
}

/// The hover tip of a status: its emoji and text. An emoji alone has nothing more to say.
fn status_tip(st: &CustomStatus) -> Option<String> {
    let text = status_text(st);
    if text.trim().is_empty() {
        return None;
    }
    Some(if st.in_meeting {
        text
    } else {
        format!("{} {text}", st.emoji).trim().to_owned()
    })
}

/// A hover tip in place of the native `title`, as in Slack: it appears at once, above an
/// element with `data-tip` (below it near the top of the window) and stays inside the window.
#[component]
fn HoverTip() -> impl IntoView {
    #[derive(Clone, PartialEq)]
    struct Tip {
        text: String,
        x: f64,
        y: f64,
        below: bool,
    }
    let tip = RwSignal::new(None::<Tip>);
    let hide = move || {
        if tip.with_untracked(Option::is_some) {
            tip.set(None);
        }
    };
    scoped_window_listener(ev::mouseover, move |e| {
        let el = e
            .target()
            .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
            .and_then(|el| el.closest("[data-tip]").ok().flatten());
        let Some((el, text)) = el.and_then(|el| el.get_attribute("data-tip").map(|t| (el, t))) else {
            return hide();
        };
        let r = el.get_bounding_client_rect();
        let below = r.top() < 48.0;
        let next = Tip {
            text,
            x: r.left() + r.width() / 2.0,
            y: if below { r.bottom() + 6.0 } else { r.top() - 6.0 },
            below,
        };
        if tip.with_untracked(|t| t.as_ref() != Some(&next)) {
            tip.set(Some(next));
        }
    });
    scoped_window_listener(ev::mouseout, move |e| {
        // The pointer left the window.
        if e.related_target().is_none() {
            hide();
        }
    });
    scoped_window_listener(ev::mousedown, move |_| hide());
    scoped_window_listener(ev::wheel, move |_| hide());
    let node = NodeRef::<html::Div>::new();
    // Shifts the tip back inside the window when the element is near its edge.
    Effect::new(move |_| {
        tip.track();
        let Some(div) = node.get() else { return };
        let div: &web_sys::HtmlElement = &div;
        let _ = div.style().set_property("margin-left", "0px");
        let r = div.get_bounding_client_rect();
        let width = window().inner_width().ok().and_then(|w| w.as_f64()).unwrap_or(0.0);
        let shift = if r.left() < 8.0 {
            8.0 - r.left()
        } else if r.right() > width - 8.0 {
            width - 8.0 - r.right()
        } else {
            0.0
        };
        let _ = div.style().set_property("margin-left", &format!("{shift:.0}px"));
    });
    move || {
        tip.get().map(|t| {
            view! {
                <div class="hover-tip" class:below=t.below node_ref=node style=format!("left:{:.0}px;top:{:.0}px", t.x, t.y)>
                    {t.text}
                </div>
            }
        })
    }
}

/// The status emoji; Telemost's "In meeting" is a green headphones icon, like a Slack huddle.
fn status_icon(st: &CustomStatus, size: u32) -> AnyView {
    if st.in_meeting {
        return view! { <span class="status-meeting"><Icon icon=I::Headphones size=size/></span> }.into_any();
    }
    let emoji = if st.emoji.is_empty() {
        "💬".to_owned()
    } else {
        st.emoji.clone()
    };
    view! { {emoji} }.into_any()
}

/// The status text; Telemost's "In meeting" comes in the UI language.
fn status_text(st: &CustomStatus) -> String {
    if st.in_meeting {
        tr().in_meeting.to_owned()
    } else {
        st.text.clone()
    }
}

fn availability_label(u: &User) -> &'static str {
    match (u.availability, u.presence) {
        (Availability::DoNotDisturb, _) => tr().dnd,
        (Availability::Busy, _) => tr().busy,
        (Availability::Away, _) => tr().away,
        (_, Presence::Online) => tr().active,
        _ => tr().offline_status,
    }
}

/// Photo URL by `AvatarId`: the public file host of the Messenger (`publicAvatarUrl`).
fn avatar_url(id: &str, size: u32) -> String {
    let size = if size > 48 { "middle" } else { "small" };
    format!("https://files.messenger.yandex.net/{id}?size={size}")
}

#[component]
pub(crate) fn Avatar(user: User, #[prop(default = 36)] size: u32, #[prop(optional)] presence: bool) -> impl IntoView {
    let radius = if size >= 32 { 8 } else { 4 };
    let style = format!(
        "width:{size}px;height:{size}px;background:{};font-size:{}px;border-radius:{radius}px",
        avatar_color(&user.id.0),
        size * 2 / 5,
    );
    let dot = presence.then(|| view! { <span class=presence_class(&user)></span> });
    // The photo goes over the initials; if it fails to load, the initials stay.
    let photo = user.avatar.clone().map(|id| {
        let failed = RwSignal::new(false);
        let src = avatar_url(&id, size);
        view! {
            <Show when=move || !failed.get()>
                <img class="avatar-img" src=src.clone() alt="" style=format!("border-radius:{radius}px")
                    on:error=move |_| failed.set(true)/>
            </Show>
        }
    });
    view! {
        <span class="avatar" style=style title=user.display_name.clone()>
            {initials(&user.display_name, size)}
            {photo}
            {dot}
        </span>
    }
}

/// Face of a chat: the peer in a direct chat, you in the chat with yourself.
fn chat_face(s: &Snapshot, ch: &Chat) -> Option<User> {
    match &ch.kind {
        ChatKind::Direct { peer } => s.user(peer).cloned(),
        ChatKind::Notes => s.user(&s.me).cloned(),
        _ => None,
    }
}

/// Chat name for the UI: your own name for the chat with yourself.
pub(crate) fn display_title(s: &Snapshot, ch: &Chat) -> String {
    match &ch.kind {
        ChatKind::Notes => s
            .user(&s.me)
            .map_or_else(|| tr().notes.to_owned(), |u| u.display_name.clone()),
        _ => chat_title(s, ch),
    }
}

/// What marks a chat in lists: its kind, or the emoji its group name starts with (`⚠️|Test`).
#[derive(Clone, PartialEq)]
struct Mark {
    kind: ChatKind,
    emoji: Option<String>,
}

impl Mark {
    fn of(ch: &Chat) -> Self {
        Mark {
            kind: ch.kind.clone(),
            emoji: ch.icon().map(str::to_owned),
        }
    }
}

fn kind_icon(mark: &Mark, size: u32) -> AnyView {
    if let Some(emoji) = &mark.emoji {
        return view! { <span class="chat-emoji" style=format!("font-size:{size}px")>{emoji.clone()}</span> }
            .into_any();
    }
    match &mark.kind {
        ChatKind::Group { public: true } => view! { <Icon icon=I::Hash size=size/> }.into_any(),
        ChatKind::Group { public: false } => view! { <Icon icon=I::Lock size=size/> }.into_any(),
        ChatKind::Broadcast => view! { <Icon icon=I::Megaphone size=size/> }.into_any(),
        ChatKind::Direct { .. } | ChatKind::Notes => view! { <Icon icon=I::Chat size=size/> }.into_any(),
    }
}

#[component]
fn Sidebar() -> impl IntoView {
    let c = ctx();
    view! {
        <aside class="sidebar" style=move || format!("width:{:.0}px", c.sidebar_w.get())>
            <div class="sb-header">
                <button class="ws-name" class:active=move || c.ctx_menu.with(|m| m.as_ref().is_some_and(|m| matches!(m.target, CtxTarget::Workspace)))
                    on:mousedown=|e| e.stop_propagation()
                    on:click=move |e| {
                        if c.ctx_menu.with_untracked(|m| m.as_ref().is_some_and(|m| matches!(m.target, CtxTarget::Workspace))) {
                            return c.ctx_menu.set(None);
                        }
                        open_menu_below(c, &e, CtxTarget::Workspace);
                    }>
                    <span>{move || org_name(c)}</span>
                    <Icon icon=I::ChevronDown size=14/>
                </button>
                <div class="sb-header-actions">
                    <button class="icon-btn sb-compose" title=tr().manage_sidebar
                        class:active=move || c.ctx_menu.with(|m| m.as_ref().is_some_and(|m| matches!(m.target, CtxTarget::SidebarPrefs)))
                        // No own mousedown-outside handling needed: a second click closes the menu itself.
                        on:mousedown=|e| e.stop_propagation()
                        on:click=move |e| {
                            if c.ctx_menu.with_untracked(|m| m.as_ref().is_some_and(|m| matches!(m.target, CtxTarget::SidebarPrefs))) {
                                return c.ctx_menu.set(None);
                            }
                            let Some(el) = e.current_target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()) else { return };
                            let r = el.get_bounding_client_rect();
                            c.ctx_menu.set(Some(CtxMenu { x: r.left(), y: r.bottom() + 4.0, target: CtxTarget::SidebarPrefs }));
                        }>
                        <Icon icon=I::Gear/>
                    </button>
                    <button class="icon-btn sb-compose" title=tr().new_message on:click=move |_| c.new_message.set(true)>
                        <Icon icon=I::Compose/>
                    </button>
                </div>
            </div>
            <Show when=move || c.view.get() == ViewMode::Dms>
                <DmList/>
            </Show>
            <Show when=move || c.view.get() == ViewMode::Activity>
                <ActivityList/>
            </Show>
            <SidebarFind/>
            <div class="sb-scroll" class:hidden=move || matches!(c.view.get(), ViewMode::Dms | ViewMode::Activity)>
                <div class="sb-quick" hidden=move || searching(c)>
                    {move || {
                        // Bold with the number of threads that have new replies, as in Slack.
                        let unread = c.threads.with(|l| l.iter().filter(|t| t.unread > 0).count());
                        let mut cls = String::from("sb-row");
                        if c.view.get() == ViewMode::Threads {
                            cls.push_str(" selected");
                        }
                        if unread > 0 {
                            cls.push_str(" unread");
                        }
                        view! {
                            <button class=cls on:click=move |_| c.view.set(ViewMode::Threads)>
                                <span class="sb-icon"><Icon icon=I::Threads size=15/></span>
                                <span class="sb-name">{tr().threads}</span>
                                {(unread > 0).then(|| view! { <span class="badge">{unread}</span> })}
                            </button>
                        }
                    }}
                    <button class=move || if c.view.get() == ViewMode::Drafts { "sb-row selected" } else { "sb-row" }
                        on:click=move |_| c.view.set(ViewMode::Drafts)>
                        <span class="sb-icon"><Icon icon=I::Send size=15/></span>
                        <span class="sb-name">{tr().drafts_and_sent}</span>
                        {move || {
                            let n = c.drafts.with(|d| d.values().filter(|v| !v.trim().is_empty()).count());
                            (n > 0).then(|| view! { <span class="sb-draft-count"><Icon icon=I::Compose size=13/>{n}</span> })
                        }}
                    </button>
                </div>
                <For
                    each=move || c.sections.get()
                    key=|sec| (sec.id.clone(), sec.title.clone(), sec.chats.clone())
                    children=move |sec: Section| view! { <SidebarSection section=sec/> }
                />
                <Show when=move || searching(c) && !any_found(c)>
                    <div class="sb-find-empty">{tr().no_conversations_found}</div>
                </Show>
            </div>
            <CallBar/>
        </aside>
    }
}

/// "Find a conversation", as in Slack: filters the sidebar sections by conversation name.
/// Enter opens the first one found, the down arrow goes to it, Escape clears the field.
#[component]
fn SidebarFind() -> impl IntoView {
    let c = ctx();
    let field = NodeRef::<html::Input>::new();
    let first_row = || {
        document()
            .query_selector(".sb-scroll .sb-row[data-chat]")
            .ok()
            .flatten()
            .and_then(|el| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(el).ok())
    };
    let keydown = move |e: ev::KeyboardEvent| match e.key().as_str() {
        "Enter" => {
            e.prevent_default();
            if let Some(row) = first_row() {
                row.click();
            }
        }
        "ArrowDown" => {
            e.prevent_default();
            if let Some(row) = first_row() {
                let _ = row.focus();
            }
        }
        "Escape" => {
            // The field's own Escape: it must not close whatever else is open.
            e.stop_propagation();
            if c.sb_query.with_untracked(String::is_empty) {
                if let Some(el) = field.get_untracked() {
                    let _ = el.blur();
                }
            } else {
                c.sb_query.set(String::new());
            }
        }
        _ => {}
    };
    view! {
        <div class="sb-find" class:hidden=move || matches!(c.view.get(), ViewMode::Dms | ViewMode::Activity)>
            <label class="sb-find-field">
                <Icon icon=I::FindList size=15/>
                <input node_ref=field type="text" placeholder=tr().find_conversation spellcheck="false" autocomplete="off"
                    prop:value=move || c.sb_query.get()
                    on:input=move |e| c.sb_query.set(event_target_value(&e))
                    on:keydown=keydown/>
                <Show when=move || searching(c)>
                    <button class="sb-find-clear" title=tr().clear_find on:click=move |_| {
                        c.sb_query.set(String::new());
                        if let Some(el) = field.get_untracked() {
                            let _ = el.focus();
                        }
                    }>
                        <Icon icon=I::X size=10/>
                    </button>
                </Show>
            </label>
        </div>
    }
}

/// "Find a conversation" has text.
fn searching(c: Ctx) -> bool {
    c.sb_query.with(|q| !q.trim().is_empty())
}

/// Whether the conversation's sidebar name has the "Find a conversation" text, in any case.
fn found(s: &Snapshot, ch: &Chat, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    let notes = (ch.kind == ChatKind::Notes)
        .then(|| s.user(&s.me).map(|u| u.display_name.to_lowercase()))
        .flatten();
    // A direct chat is found by the person's name and by the chat's own name from Telemost too.
    chat_title(s, ch).to_lowercase().contains(&query)
        || ch.name.to_lowercase().contains(&query)
        || notes.is_some_and(|n| n.contains(&query))
}

/// Whether "Find a conversation" finds anything in the sidebar sections.
fn any_found(c: Ctx) -> bool {
    c.sb_query.with(|q| {
        c.sections.with(|list| {
            c.snap.with(|s| {
                list.iter()
                    .flat_map(|sec| sec.chats.iter())
                    .filter_map(|id| s.chat(id))
                    .any(|ch| in_current_org(c, ch) && !c.hidden.with(|h| hidden::hides(h, ch)) && found(s, ch, q))
            })
        })
    })
}

/// The current call at the bottom of the sidebar, like the Slack huddle bar: where it is and "Leave".
#[component]
fn CallBar() -> impl IntoView {
    let c = ctx();
    move || {
        c.call.get().map(|call| {
            let chat = call.chat_id.clone().map(ChatId);
            let open_chat = move |_| {
                if let Some(id) = chat.clone() {
                    ops::open_chat(c, id);
                }
            };
            view! {
                <div class="call-bar">
                    <button class="call-where" title=tr().call_go_to_chat on:click=open_chat>
                        <span class="call-live"><Icon icon=I::Headphones size=14/></span>
                        <span class="call-text">
                            <span class="call-kind">{tr().call_in_progress}</span>
                            <span class="call-label">{call.label.clone()}</span>
                        </span>
                    </button>
                    <button class="call-btn leave" title=tr().call_leave on:click=move |_| ops::leave_call(c)>
                        <Icon icon=I::PhoneOff size=16/>
                        <span>{tr().call_leave}</span>
                    </button>
                </div>
            }
        })
    }
}

#[component]
fn SidebarSection(section: Section) -> impl IntoView {
    let c = ctx();
    let collapsed = {
        let id = section.id.clone();
        Signal::derive(move || c.collapsed.with(|s| s.contains(&id)))
    };
    let toggle = {
        let id = section.id.clone();
        move |_| {
            c.collapsed.update(|s| {
                if !s.remove(&id) {
                    s.insert(id.clone());
                }
            })
        }
    };
    let section_id = section.id.clone();
    // Section conversations after the sidebar filter and sorting.
    let visible = {
        let (chats, section_id) = (section.chats.clone(), section.id.clone());
        Memo::new(move |_| {
            let (filter, hide_muted) = c.settings.with(|s| (s.sidebar_filter, s.hide_muted));
            let query = c.sb_query.with(|q| q.trim().to_owned());
            let current = chat_in_pane(c);
            let now = (js_sys::Date::now() * 1000.0) as Timestamp;
            c.snap.with(|s| {
                section_order(c, s, &section_id, &chats)
                    .iter()
                    .filter_map(|id| s.chat(id))
                    .filter(|ch| {
                        // "Find a conversation" looks through all of them, the filter aside.
                        if !query.is_empty() {
                            return found(s, ch, &query);
                        }
                        // The filter never hides starred chats and the chat with yourself: they are always on top.
                        section_id == "starred"
                            || current.as_ref() == Some(&ch.id)
                            || shown_in_sidebar(
                                ch,
                                filter,
                                hide_muted,
                                now,
                                c.drafts.with(|d| d.get(&ch.id).is_some_and(|t| !t.trim().is_empty())),
                            )
                    })
                    .filter(|ch| in_current_org(c, ch))
                    .filter(|ch| !c.hidden.with(|h| hidden::hides(h, ch)))
                    .map(|ch| ch.id.clone())
                    .collect::<Vec<_>>()
            })
        })
    };
    let always = section_id == "channels" || section_id == "dms" || folders::is_folder(&section_id);
    // An empty section, or one whose chats are elsewhere in the list, takes a dragged chat whole.
    let drop_into = {
        let id = section_id.clone();
        Memo::new(move |_| {
            c.chat_drag.with(|d| {
                d.as_ref().is_some_and(|d| {
                    d.active && !d.dropped && d.target.as_ref().is_some_and(|t| t.section == id && t.anchor.is_none())
                })
            })
        })
    };
    let empty = Memo::new(move |_| visible.with(Vec::is_empty));
    // The section icon, as in Slack: a star for "Starred", a hash for "Channels", speech bubbles for
    // "Direct messages", the emoji of a folder; "Meeting chats" and folders without an emoji have none.
    let icon = match section_id.as_str() {
        "starred" => Some(I::Star),
        "channels" => Some(I::HashSquare),
        "dms" => Some(I::Messages),
        _ => None,
    };
    let emoji = {
        let id = section_id.clone();
        Memo::new(move |_| {
            c.folders
                .with(|f| f.iter().find(|f| f.id == id).and_then(|f| f.emoji.clone()))
        })
    };
    let has_icon = Memo::new(move |_| icon.is_some() || emoji.with(Option::is_some));
    // The "+" on hover, as in Slack: a channel in "Channels" and in one's own sections (it goes
    // into that section), a message in "Direct messages".
    let add = match section_id.as_str() {
        "channels" => Some(SectionAdd::Channel(None)),
        "dms" => Some(SectionAdd::Message),
        id if folders::is_folder(id) => Some(SectionAdd::Channel(Some(id.to_owned()))),
        _ => None,
    };
    move || {
        ((always && !searching(c)) || !empty.get()).then(|| {
        let section_id = section_id.clone();
        view! {
            <div class="sb-section" class:drop-into=move || drop_into.get() data-section=section_id.clone()>
                <div class="sb-section-head" class:menu-open={
                    let id = section_id.clone();
                    move || c.ctx_menu.with(|m| matches!(m, Some(CtxMenu { target: CtxTarget::Section(s), .. }) if *s == id))
                }>
                // As in Slack, no caret until hovered: then it shows in place of the section icon
                // (or left of an icon-less title) and turns down over an expanded section.
                <button class="sb-section-title" class:with-icon=move || has_icon.get()
                    aria-expanded=move || if collapsed.get() { "false" } else { "true" }
                    on:click=toggle.clone() on:contextmenu={
                    let id = section.id.clone();
                    move |e: ev::MouseEvent| {
                        e.prevent_default();
                        c.ctx_menu.set(Some(CtxMenu { x: e.client_x() as f64, y: e.client_y() as f64, target: CtxTarget::Section(id.clone()) }));
                    }
                }>
                    <span class="sb-section-slot">
                        {icon.map(|i| view! { <span class="sb-section-icon"><Icon icon=i size=15/></span> })}
                        {move || emoji.get().map(|e| view! { <span class="sb-section-icon emoji">{e}</span> })}
                        <span class="caret" class:collapsed=move || collapsed.get()>
                            <Icon icon=I::ChevronDown size=12/>
                        </span>
                    </span>
                    <span class="sb-section-name">{tr().section(&section.id, &section.title)}</span>
                </button>
                // "+" and "⋮" on hover, as in Slack: they work even when the section is collapsed.
                <div class="sb-section-tools">
                    {add.clone().map(|add| {
                        let title = match add { SectionAdd::Message => tr().new_message, SectionAdd::Channel(_) => tr().create_channel };
                        view! {
                            <button class="sb-section-tool" title=title on:click=move |_| match &add {
                                SectionAdd::Message => c.new_message.set(true),
                                SectionAdd::Channel(folder) => c.creating_channel.set(Some(NewChannel { folder: folder.clone() })),
                            }>
                                <Icon icon=I::Plus size=14/>
                            </button>
                        }
                    })}
                    <button class="sb-section-tool" title=tr().section_options on:mousedown=|e| e.stop_propagation() on:click={
                        let id = section.id.clone();
                        move |e| open_menu_below(c, &e, CtxTarget::Section(id.clone()))
                    }>
                        <Icon icon=I::Kebab size=14/>
                    </button>
                </div>
                </div>
                {let section_id = section_id.clone(); move || {
                    // A collapsed section still shows chats with unread messages, the open chat included only
                    // when it has them. Found conversations show in collapsed sections too.
                    let folded = collapsed.get() && !searching(c);
                    visible
                        .get()
                        .into_iter()
                        .filter(|id| {
                            !folded || c.snap.with(|s| s.chat(id).is_some_and(|ch| ch.unread > 0 && !ch.muted))
                        })
                        .map(|id| view! { <ChatRow id=id section=section_id.clone()/> })
                        .collect_view()
                }}
                {(section_id == "dms").then(|| view! {
                    <Show when=move || !collapsed.get() && !searching(c)>
                        <button class="sb-row sb-add" on:click=move |_| c.new_message.set(true)>
                            <span class="sb-icon add"><Icon icon=I::Plus size=14/></span>
                            <span class="sb-name">{tr().write_to_colleague}</span>
                        </button>
                    </Show>
                })}
                {(section_id == "channels").then(|| view! {
                    <Show when=move || !collapsed.get() && !searching(c)>
                        <button class="sb-row sb-add" on:mousedown=|e| e.stop_propagation() on:click=move |e| open_menu_below(c, &e, CtxTarget::AddChannels)>
                            <span class="sb-icon add"><Icon icon=I::Plus size=14/></span>
                            <span class="sb-name">{tr().add_channels}</span>
                        </button>
                    </Show>
                })}
            </div>
        }
    })
    }
}

/// What the "+" of a section header starts.
#[derive(Clone)]
enum SectionAdd {
    /// A channel; from one's own section it goes into that section.
    Channel(Option<String>),
    /// A direct message.
    Message,
}

/// Section chats in sidebar order, before the filter. Starred chats keep the Telemost order
/// unless the user dragged them; chats new to a custom order go on top (in "Starred", at the bottom).
fn section_order(c: Ctx, s: &Snapshot, section_id: &str, chats: &[ChatId]) -> Vec<ChatId> {
    let starred = section_id == "starred";
    // A folder keeps its own order in place of the custom one.
    if folders::is_folder(section_id) {
        let mut list = chats.to_vec();
        match c.settings.with(|st| st.sidebar_sort) {
            SidebarSort::Alphabetical => {
                list.sort_by_cached_key(|id| s.chat(id).map(|ch| chat_title(s, ch).to_lowercase()))
            }
            SidebarSort::Recency => list.sort_by_key(|id| std::cmp::Reverse(s.chat(id).map_or(0, |ch| ch.last_ts))),
            SidebarSort::Custom => {}
        }
        return list;
    }
    match c.settings.with(|st| st.sidebar_sort) {
        SidebarSort::Alphabetical if !starred => {
            let mut list = chats.to_vec();
            list.sort_by_cached_key(|id| {
                s.chat(id)
                    .map(|ch| (ch.kind != ChatKind::Notes, chat_title(s, ch).to_lowercase()))
            });
            list
        }
        SidebarSort::Custom => c
            .chat_order
            .with(|o| order::apply(o.get(section_id).map_or(&[], Vec::as_slice), chats, starred)),
        _ => chats.to_vec(),
    }
}

/// Whether a chat dragged out of its section may be dropped into section `to`: its own section
/// (a new place there), a folder, or from a folder back to the Telemost section of its kind.
/// "Starred" is Telemost's own and neither takes nor gives chats.
fn accepts(c: Ctx, d: &ChatDrag, to: &str) -> bool {
    if to == d.section {
        return true;
    }
    if d.section == "starred" || to == "starred" {
        return false;
    }
    let apart = c.settings.with_untracked(|s| s.group_calls_apart);
    folders::is_folder(to)
        || (folders::is_folder(&d.section)
            && c.snap
                .with_untracked(|s| s.chat(&d.chat).map(|ch| folders::shown_section(ch, apart)))
                == Some(to))
}

/// Where a dragged chat would land: the section under the pointer and, among its rows, next to
/// the row under the pointer, before or after it. Back to its Telemost section it goes whole:
/// the place there is up to that section's order.
fn drop_target(c: Ctx, d: &ChatDrag, y: f64) -> Option<DropAt> {
    let as_element = |n: web_sys::Node| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(n).ok();
    let sections = document()
        .query_selector_all(".sb-scroll .sb-section[data-section]")
        .ok()?;
    let el = (0..sections.length())
        .filter_map(|i| sections.item(i).and_then(as_element))
        .find(|el| {
            let r = el.get_bounding_client_rect();
            y >= r.top() && y < r.bottom()
        })?;
    let section = el.get_attribute("data-section")?;
    if !accepts(c, d, &section) {
        return None;
    }
    if section != d.section && !folders::is_folder(&section) {
        return Some(DropAt { section, anchor: None });
    }
    let rows = el.query_selector_all(".sb-row[data-chat]").ok()?;
    let mut anchor = None;
    for el in (0..rows.length()).filter_map(|i| rows.item(i).and_then(as_element)) {
        let Some(id) = el.get_attribute("data-chat").map(ChatId) else {
            continue;
        };
        let r = el.get_bounding_client_rect();
        if y < r.top() + r.height() / 2.0 {
            anchor = Some((id, false));
            break;
        }
        anchor = Some((id, true));
    }
    Some(DropAt { section, anchor })
}

/// Pointer move with a sidebar chat held: starts the drag after a few pixels, then tracks the drop place
/// and scrolls the sidebar near its edges.
fn chat_drag_to(c: Ctx, e: &ev::MouseEvent) {
    let Some(d) = c.chat_drag.get_untracked() else { return };
    if d.dropped {
        return;
    }
    // The button was released outside the window.
    if e.buttons() & 1 == 0 {
        return c.chat_drag.set(None);
    }
    let (x, y) = (e.client_x() as f64, e.client_y() as f64);
    if !d.active && (x - d.from.0).hypot(y - d.from.1) < 5.0 {
        return;
    }
    if let Some(el) = document().query_selector(".sb-scroll").ok().flatten() {
        let r = el.get_bounding_client_rect();
        if y < r.top() + 24.0 {
            el.set_scroll_top(el.scroll_top() - 12);
        } else if y > r.bottom() - 24.0 {
            el.set_scroll_top(el.scroll_top() + 12);
        }
    }
    let target = drop_target(c, &d, y);
    if !d.active || d.target != target {
        c.chat_drag.set(Some(ChatDrag {
            active: true,
            target,
            ..d
        }));
    }
}

/// Button released: puts the dragged chat in its new place and switches the sidebar to the custom order.
fn chat_drop(c: Ctx) {
    let Some(d) = c.chat_drag.get_untracked() else { return };
    if !d.active {
        return c.chat_drag.set(None);
    }
    if d.dropped {
        return;
    }
    c.chat_drag.set(Some(ChatDrag {
        dropped: true,
        target: None,
        ..d.clone()
    }));
    set_timeout(move || c.chat_drag.set(None), Duration::ZERO);
    let Some(DropAt { section: to, anchor }) = d.target else {
        return;
    };
    // Into a folder, or out of one back home.
    if to != d.section {
        let into = folders::is_folder(&to).then_some(to.as_str());
        let anchor = anchor.as_ref().map(|(id, after)| (id, *after));
        ops::change_folders(c, |f| folders::move_chat(f, &d.chat, into, anchor));
        return;
    }
    let Some((anchor, after)) = anchor else { return };
    let current = untrack(|| {
        c.sections.with(|list| {
            list.iter()
                .find(|sec| sec.id == d.section)
                .map(|sec| c.snap.with(|s| section_order(c, s, &sec.id, &sec.chats)))
        })
    });
    let Some(current) = current else { return };
    let moved = order::move_chat(current.clone(), &d.chat, &anchor, after);
    if moved == current {
        return;
    }
    // The switch to the custom order applies to every section: the others keep the order they
    // are shown in now (say, A–Z), not a custom order saved long ago nor the Telemost one.
    let switching = c.settings.with_untracked(|s| s.sidebar_sort != SidebarSort::Custom);
    let shown: Vec<(String, Vec<ChatId>, bool)> = if switching {
        untrack(|| {
            c.sections.with(|list| {
                c.snap.with(|s| {
                    list.iter()
                        .filter(|sec| sec.id != d.section)
                        .map(|sec| {
                            let order = section_order(c, s, &sec.id, &sec.chats);
                            // Without a saved order the custom one is the Telemost order, `sec.chats`.
                            let telemost = order == sec.chats;
                            (sec.id.clone(), order, telemost)
                        })
                        .collect()
                })
            })
        })
    } else {
        Vec::new()
    };
    // A folder's order is its own list; the Telemost sections keep theirs in `chat_order`.
    let (in_folders, in_sections): (Vec<_>, Vec<_>) = std::iter::once((d.section.clone(), moved, false))
        .chain(shown)
        .partition(|(id, _, _)| folders::is_folder(id));
    if !in_folders.is_empty() {
        ops::change_folders(c, |f| {
            for (id, order, _) in &in_folders {
                folders::reorder(f, id, order);
            }
        });
    }
    c.chat_order.update(|o| {
        for (id, order, telemost) in in_sections {
            if telemost && id != d.section {
                o.remove(&id);
            } else {
                o.insert(id, order);
            }
        }
    });
    if switching {
        c.settings.update(|s| s.sidebar_sort = SidebarSort::Custom);
    }
}

/// The chat shown in the main pane, highlighted in the sidebar. `current` keeps the last opened
/// chat while "Threads" or "Drafts & sent" is shown in its place: then no chat is open.
fn chat_in_pane(c: Ctx) -> Option<ChatId> {
    (!c.view.get().is_home_page()).then(|| c.current.get())
}

/// A conversation without new messages for longer than this is inactive ("Active only" in Slack).
const INACTIVE_AFTER_MCS: Timestamp = 30 * 24 * 3600 * 1_000_000;

/// Whether to show a conversation in the sidebar with the selected filter. Mentions are always visible,
/// as are conversations with a draft; a last message time of 0 means "unknown" and the conversation is not hidden.
fn shown_in_sidebar(ch: &Chat, filter: SidebarFilter, hide_muted: bool, now: Timestamp, has_draft: bool) -> bool {
    if ch.mentions > 0 || ch.kind == ChatKind::Notes {
        return true;
    }
    if hide_muted && ch.muted {
        return false;
    }
    let unread = ch.unread > 0 && !ch.muted;
    match filter {
        SidebarFilter::All => true,
        SidebarFilter::Active => {
            unread || has_draft || ch.last_ts == 0 || now.saturating_sub(ch.last_ts) <= INACTIVE_AFTER_MCS
        }
        SidebarFilter::Unreads => unread,
        SidebarFilter::Mentions => false,
    }
}

#[component]
fn ChatRow(id: ChatId, section: String) -> impl IntoView {
    let c = ctx();
    let data = {
        let id = id.clone();
        Memo::new(move |_| {
            c.snap.with(|s| {
                s.chat(&id).map(|ch| {
                    let face = match &ch.kind {
                        ChatKind::Direct { peer } => s.user(peer).cloned(),
                        ChatKind::Notes => s.user(&s.me).cloned(),
                        _ => None,
                    };
                    let title = match (&ch.kind, &face) {
                        (ChatKind::Notes, Some(me)) => me.display_name.clone(),
                        _ => chat_title(s, ch),
                    };
                    (ch.clone(), title, face)
                })
            })
        })
    };
    let selected = {
        let id = id.clone();
        Memo::new(move |_| chat_in_pane(c).as_ref() == Some(&id))
    };
    let ctx_open = {
        let id = id.clone();
        Memo::new(move |_| c.ctx_menu.with(|m| m.as_ref().is_some_and(|m| m.is_chat(&id))))
    };
    let on_ctx = {
        let id = id.clone();
        move |e: ev::MouseEvent| {
            e.prevent_default();
            c.ctx_menu.set(Some(CtxMenu {
                x: e.client_x() as f64,
                y: e.client_y() as f64,
                target: CtxTarget::Chat(id.clone()),
            }));
        }
    };
    // Drag state of this row: being dragged, or the drop place before or after it.
    let drag_cls = {
        let id = id.clone();
        Memo::new(move |_| {
            c.chat_drag.with(|d| match d {
                Some(d) if d.active && !d.dropped => {
                    if d.chat == id {
                        " dragging"
                    } else {
                        match d.target.as_ref().and_then(|t| t.anchor.as_ref()) {
                            Some((t, false)) if t == &id => " drop-before",
                            Some((t, true)) if t == &id => " drop-after",
                            _ => "",
                        }
                    }
                }
                _ => "",
            })
        })
    };
    let hold = {
        let id = id.clone();
        move |e: ev::MouseEvent| {
            if e.button() != 0 {
                return;
            }
            // No text selection or native image drag: the row is dragged by hand.
            e.prevent_default();
            c.chat_drag.set(Some(ChatDrag {
                section: section.clone(),
                chat: id.clone(),
                from: (e.client_x() as f64, e.client_y() as f64),
                active: false,
                target: None,
                dropped: false,
            }));
        }
    };
    let data_chat = id.0.clone();
    // Someone is typing in the chat: three dots on the right, the names in the tooltip.
    let typing = {
        let id = id.clone();
        Memo::new(move |_| tr().typing(&typers(c, &id)))
    };
    let typing_mark = move || {
        typing.get().map(|hint| {
            view! { <span class="sb-typing" title=hint><i></i><i></i><i></i></span> }
        })
    };
    let select = move |_| {
        // The click that ends a drag.
        if c.chat_drag.with_untracked(|d| d.as_ref().is_some_and(|d| d.active)) {
            return;
        }
        if c.view.get_untracked().is_home_page() {
            c.view.set(ViewMode::Home);
        }
        ops::open_chat(c, id.clone());
    };
    move || {
        data.get().map(|(ch, title, face)| {
            let mut cls = String::from("sb-row");
            if selected.get() {
                cls.push_str(" selected");
            }
            if ch.unread > 0 && !ch.muted {
                cls.push_str(" unread");
            }
            if ch.muted {
                cls.push_str(" muted");
            }
            if ctx_open.get() {
                cls.push_str(" ctx");
            }
            cls.push_str(drag_cls.get());
            let icon = match face {
                Some(u) => {
                    view! { <span class="sb-icon face"><Avatar user=u size=20 presence=true/></span> }.into_any()
                }
                None => view! { <span class="sb-icon">{kind_icon(&Mark::of(&ch), 15)}</span> }.into_any(),
            };
            let you =
                matches!(ch.kind, ChatKind::Notes).then(|| view! { <span class="sb-you">{tr().you_short}</span> });
            let live = c.active_calls.with(|a| a.contains_key(&ch.id));
            // A call in a direct chat shows once: the person's "In meeting" is that very call.
            let face_status = c.snap.with(|s| {
                let user = match &ch.kind {
                    ChatKind::Direct { peer } => s.user(peer),
                    ChatKind::Notes => s.user(&s.me),
                    _ => None,
                }?;
                let in_this_call = live && user.status.as_ref().is_some_and(|st| st.in_meeting);
                if in_this_call { None } else { status_badge(user) }
            });
            let badge = (ch.mentions > 0).then(|| view! { <span class="badge">{ch.mentions}</span> });
            let live_call = live.then(|| {
                view! { <span class="sb-call" title=tr().call_live_hint><Icon icon=I::Headphones size=13/></span> }
            });
            let has_draft = !selected.get() && c.drafts.with(|d| d.get(&ch.id).is_some_and(|t| !t.trim().is_empty()));
            let pencil = (has_draft && ch.mentions == 0)
                .then(|| view! { <span class="sb-pencil"><Icon icon=I::Compose size=13/></span> });
            // "×" on hover hides the chat, as closing a conversation in Slack. Not a nested button:
            // the row is one already.
            let close = (ch.kind != ChatKind::Notes).then(|| {
                let id = ch.id.clone();
                view! {
                    <span class="sb-close" role="button" title=tr().hide_chat
                        on:mousedown=|e| e.stop_propagation()
                        on:click=move |e| {
                            e.stop_propagation();
                            ops::hide_chat(c, id.clone());
                        }>
                        <Icon icon=I::X size=12/>
                    </span>
                }
            });
            view! {
                <button class=cls data-chat=data_chat.clone() on:mousedown=hold.clone() on:click=select.clone() on:contextmenu=on_ctx.clone()>
                    {icon}
                    <span class="sb-name">{title}</span>
                    {you}
                    {face_status}
                    {typing_mark}
                    {live_call}
                    {pencil}
                    {badge}
                    {close}
                </button>
            }
        })
    }
}

#[component]
fn ChannelHeader() -> impl IntoView {
    let c = ctx();
    let info = Memo::new(move |_| {
        let id = c.current.get();
        c.snap.with(|s| {
            s.chat(&id)
                .map(|ch| (ch.clone(), display_title(s, ch), chat_face(s, ch)))
        })
    });
    view! {
        <header class="ch-header">
            <div class="ch-title-row">
                <div class="ch-title">
                    {move || info.get().map(|(ch, title, face)| match face {
                        // In a direct chat the name opens the peer's profile, as in Slack.
                        Some(u) => {
                            let id = u.id.clone();
                            let status = status_badge(&u);
                            view! {
                                <button class="ch-person" on:click=move |_| ops::open_profile(c, id.clone())>
                                    <span class="ch-face"><Avatar user=u size=24 presence=true/></span>
                                    <span>{title}</span>
                                    {status}
                                </button>
                            }
                            .into_any()
                        }
                        None if matches!(ch.kind, ChatKind::Group { .. }) => view! {
                            <button class="ch-person" title=tr().about_channel on:click=move |_| c.details.set(Some(DetailsTab::About))>
                                <span class="ch-kind">{kind_icon(&Mark::of(&ch), 18)}</span><span>{title}</span><Icon icon=I::ChevronDown size=14/>
                            </button>
                        }
                        .into_any(),
                        None => view! { <span class="ch-kind">{kind_icon(&Mark::of(&ch), 18)}</span><span>{title}</span> }.into_any(),
                    })}
                </div>
                <div class="ch-actions">
                    {move || info.get().filter(|(ch, _, _)| !matches!(ch.kind, ChatKind::Direct { .. } | ChatKind::Notes)).map(|(ch, _, _)| view! {
                        <button class="ch-members" title=tr().members on:click=move |_| c.details.set(Some(DetailsTab::Members))>
                            <Icon icon=I::Users size=15/>
                            <span>{ch.member_count}</span>
                        </button>
                    })}
                    {move || {
                        // A call going on in the chat turns the button into "Join", as in Slack,
                        // unless the user is already in it.
                        let current = c.current.get();
                        let joined = c.call.with(|k| k.as_ref().is_some_and(|k| k.chat_id.as_deref() == Some(current.0.as_str())));
                        let live = !joined && c.active_calls.with(|a| a.contains_key(&current));
                        let title = match (live, in_call(c, &current)) {
                            (true, Some(n)) => format!("{} · {}", tr().call_live_hint, tr().in_call(n)),
                            (true, None) => tr().call_live_hint.to_owned(),
                            _ => tr().call_hint.to_owned(),
                        };
                        view! {
                            <button class="huddle-btn" class:live=live class:active=joined
                                title=title
                                on:click=move |_| ops::start_call(c, c.current.get_untracked())>
                                {if live {
                                    view! { <Icon icon=I::Headphones size=16/> }.into_any()
                                } else {
                                    view! { <Icon icon=I::Video size=16/> }.into_any()
                                }}
                                <span>{if live { tr().call_join } else { tr().call }}</span>
                            </button>
                        }
                    }}
                </div>
            </div>
            <nav class="ch-tabs">
                <button class=move || if c.pins_tab.get() { "tab" } else { "tab active" } on:click=move |_| c.pins_tab.set(false)>
                    <Icon icon=I::Chat size=14/><span>{tr().tab_messages}</span>
                </button>
                // Like the "Pins" tab in Slack: only if the chat has a pinned message.
                {move || info.get().and_then(|(ch, _, _)| ch.pinned).map(|_| view! {
                    <button class=move || if c.pins_tab.get() { "tab active" } else { "tab" } on:click=move |_| c.pins_tab.set(true)>
                        <Icon icon=I::Pin size=14/><span>{tr().tab_pins}</span>
                    </button>
                })}
            </nav>
        </header>
    }
}

/// The "X is typing…" line for a chat.
fn typing_line(c: Ctx, chat: &ChatId) -> Option<String> {
    tr().typing(&typers(c, chat))
}

/// Names of the people typing in a chat now.
fn typers(c: Ctx, chat: &ChatId) -> Vec<String> {
    let now = c.now.get();
    c.typing.with(|t| {
        if !t.iter().any(|(ch, _, until)| ch == chat && *until > now) {
            return vec![];
        }
        c.snap.with(|s| {
            t.iter()
                .filter(|(ch, _, until)| ch == chat && *until > now)
                .map(|(_, u, _)| {
                    s.user(u)
                        .map_or_else(|| tr().someone.to_owned(), |u| u.display_name.clone())
                })
                .collect()
        })
    })
}

#[component]
fn MessageList() -> impl IntoView {
    let c = ctx();
    let list_ref = NodeRef::<html::Div>::new();
    let messages = Memo::new(move |_| {
        let id = c.current.get();
        c.snap.with(|s| s.channel_messages(&id))
    });
    let chat = Memo::new(move |_| {
        let id = c.current.get();
        c.snap.with(|s| s.chat(&id).cloned())
    });
    // The read line: a reading of the chat moves it, and only the lines it leaves and reaches change.
    let read_line = Memo::new(move |_| {
        if !c.settings.with(|s| s.show_read_receipts) {
            return None;
        }
        let me = c.snap.with_untracked(|s| s.me.clone());
        chat.with(|ch| {
            let ch = ch.as_ref()?;
            messages.with(|list| receipts::read_line(ch, &me, list))
        })
    });
    // Scrolling: to the bottom when the chat changes or new messages arrive, if the user is at the bottom;
    // when older messages load, keep the visible position.
    let stick = RwSignal::new(true);
    let anchor: RwSignal<Option<(i32, i32)>> = RwSignal::new(None);
    Effect::new(move |prev: Option<ChatId>| {
        let id = c.current.get();
        if prev.as_ref() != Some(&id) {
            stick.set(true);
        }
        id
    });
    Effect::new(move |_| {
        messages.track();
        // The list may be gone by the next frame (another view opened).
        request_animation_frame(move || {
            let Some(el) = list_ref.try_get_untracked().flatten() else {
                return;
            };
            if let Some((height, top)) = anchor.try_get_untracked().flatten() {
                el.set_scroll_top(el.scroll_height() - height + top);
                anchor.try_set(None);
            } else if stick.try_get_untracked() == Some(true) {
                el.set_scroll_top(el.scroll_height());
            }
        });
    });
    Effect::new(move |_| {
        if c.highlight.with(Option::is_some) {
            stick.set(false);
        }
    });
    // Pictures and videos get their height only once loaded, after the list has scrolled down:
    // stay at the bottom while they come. `load` does not bubble, so it is caught on the way down.
    Effect::new(move |attached: Option<bool>| {
        if attached == Some(true) {
            return true;
        }
        let Some(el) = list_ref.get() else { return false };
        let on_load = wasm_bindgen::closure::Closure::<dyn Fn()>::new(move || {
            if stick.try_get_untracked() == Some(true)
                && anchor.try_get_untracked().is_some_and(|a| a.is_none())
                && let Some(el) = list_ref.try_get_untracked().flatten()
            {
                el.set_scroll_top(el.scroll_height());
            }
        });
        for event in ["load", "loadedmetadata"] {
            let _ = el.add_event_listener_with_callback_and_bool(
                event,
                wasm_bindgen::JsCast::unchecked_ref(on_load.as_ref()),
                true,
            );
        }
        // Lives as long as the element; the signals are checked with `try_` after unmount.
        on_load.forget();
        true
    });
    let on_scroll = move |_| {
        let Some(el) = list_ref.get_untracked() else { return };
        stick.set(el.scroll_height() - el.scroll_top() - el.client_height() < 120);
        if el.scroll_top() < 200 && ops::load_older(c) {
            anchor.set(Some((el.scroll_height(), el.scroll_top())));
        }
    };
    view! {
        <div class="messages chat-feed" node_ref=list_ref on:scroll=on_scroll>
            {move || {
                let id = c.current.get();
                let done = !c.live || c.exhausted.with(|e| e.contains(&id));
                if done {
                    view! { <ChannelIntro/> }.into_any()
                } else if c.loading_older.get() {
                    view! { <div class="loading-older"><div class="spinner small"></div></div> }.into_any()
                } else {
                    view! { <div class="loading-older"></div> }.into_any()
                }
            }}
            <Feed messages=messages read_line=read_line/>
        </div>
    }
}

/// Text typed in the editor of a message and where the caret was.
pub struct EditDraft {
    key: (ChatId, Timestamp),
    text: String,
    caret: Option<u32>,
}

/// In-place message editing, as in Slack: a text field, "Cancel" and "Save".
/// Enter saves (or Ctrl+Enter, depending on the preference), Esc cancels.
#[component]
fn InlineEditor(msg: Message) -> impl IntoView {
    let c = ctx();
    // Mentions are edited by name and sent back as ids.
    let (shown, named) = c.snap.with_untracked(|s| {
        s.mention_names()
            .with(|who| mostik_core::markup::ids_to_names(&msg.text, who))
    });
    c.picked
        .update(|p| p.extend(named.into_iter().map(|(name, id)| (name, UserId(id)))));
    let original = shown.trim().to_owned();
    // The line may have been built again while the user was typing: go on with their draft.
    let key = (msg.chat_id.clone(), msg.ts);
    let draft = c
        .edit_draft
        .with_value(|d| d.as_ref().filter(|d| d.key == key).map(|d| (d.text.clone(), d.caret)));
    let (initial, caret) = draft.unwrap_or((shown, None));
    let caret = caret.unwrap_or(utf16_len(&initial));
    let text = RwSignal::new(initial);
    let busy = RwSignal::new(false);
    let area = NodeRef::<html::Div>::new();
    Effect::new(move |first: Option<()>| {
        let text = text.get();
        if first.is_none() {
            return;
        }
        let caret = area.get_untracked().and_then(|el| selection(&el).1);
        c.edit_draft.set_value(Some(EditDraft {
            key: key.clone(),
            text,
            caret,
        }));
    });
    let initial = text.get_untracked();
    Effect::new(move |_| {
        if let Some(el) = area.get() {
            // Only the field appearing fills it: the emoji lookup reads the snapshot, and its
            // changes (someone came online) would put the original text back over the typed one.
            untrack(|| place_caret(&el, &initial, caret));
            // The editor is taller than the message: the last message's editor would go under
            // the composer. Once laid out, scroll it fully into view.
            request_animation_frame(move || {
                if let Some(el) = area.try_get_untracked().flatten() {
                    reveal_editor(&el);
                }
            });
        }
    });
    let cancel = move || c.editing.set(None);
    let save = {
        let msg = msg.clone();
        move || {
            let t = text.get_untracked().trim().to_owned();
            if busy.get_untracked() {
                return;
            }
            if t.is_empty() {
                // As in Slack: empty text offers to delete the message.
                c.editing.set(None);
                return c.confirm_delete.set(Some(msg.clone()));
            }
            if t == original {
                return c.editing.set(None);
            }
            busy.set(true);
            let m = msg.clone();
            spawn_local(async move {
                match ops::edit(c, m, t).await {
                    Ok(()) => c.editing.set(None),
                    Err(e) => c.error.set(Some(format!("{}: {e}", tr().err_not_sent))),
                }
                // Saved, the editor closes and its signals go.
                busy.try_set(false);
            });
        }
    };
    let save_key = save.clone();
    let field = crate::compose::Field::new(area, move |t| text.set(t));
    view! {
        <div class="inline-edit" on:click=|e| e.stop_propagation()>
            <div class="inline-edit-box">
                <div class="editor" node_ref=area contenteditable="plaintext-only" role="textbox" aria-multiline="true"
                    on:input=move |_| {
                        if let Some(el) = area.get_untracked() {
                            text.set(crate::editor::tidy(&el, &crate::custom::lookup(c)));
                            reveal_editor(&el);
                        }
                    }
                    on:paste=move |e: ev::ClipboardEvent| { field.paste_link(&e); }
                    on:keydown=move |e: ev::KeyboardEvent| {
                        if e.is_composing() || field.hotkey(&e) { return; }
                        match e.key().as_str() {
                            "Escape" => { e.prevent_default(); e.stop_propagation(); cancel(); }
                            "Enter" if (enter_sends() && !e.shift_key() && !e.ctrl_key()) || (!enter_sends() && e.ctrl_key()) => {
                                e.prevent_default();
                                save_key();
                            }
                            "Enter" => {
                                e.prevent_default();
                                if let Some(el) = area.get_untracked() {
                                    text.set(insert_text(&el, "\n", 0));
                                }
                            }
                            _ => {}
                        }
                    }></div>
                <crate::compose::FormatBar field emoji=true/>
            </div>
            <div class="inline-edit-actions">
                <button class="btn small" on:click=move |_| cancel()>{tr().cancel}</button>
                <button class="btn small primary" disabled=move || busy.get() on:click=move |_| save()>
                    <Icon icon=I::Enter size=14/>{tr().save_changes}
                </button>
            </div>
        </div>
    }
}

/// Scrolls the message list just enough for the whole in-place editor (text and buttons) to show,
/// with the gap from `scroll-margin` in the CSS.
fn reveal_editor(area: &web_sys::HtmlElement) {
    if let Ok(Some(editor)) = area.closest(".inline-edit") {
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
        editor.scroll_into_view_with_scroll_into_view_options(&opts);
    }
}

#[component]
fn PinsView() -> impl IntoView {
    let c = ctx();
    let data = Memo::new(move |_| {
        let id = c.current.get();
        c.snap.with(|s| {
            let ts = s.chat(&id)?.pinned?;
            Some((
                id.clone(),
                ts,
                s.messages.iter().find(|m| m.chat_id == id && m.ts == ts).cloned(),
            ))
        })
    });
    // If the message is not loaded yet, load history up to it.
    Effect::new(move |_| {
        if let Some((id, ts, None)) = data.get() {
            ops::jump_to_message(c, id, ts, None);
            c.pins_tab.set(true);
        }
    });
    view! {
        <div class="messages pins-list">
            {move || match data.get() {
                None => view! { <div class="empty-state"><h3>{tr().no_pins}</h3></div> }.into_any(),
                Some((id, ts, Some(m))) => view! {
                    <div class="pin-card">
                        <MessageRow msg=m grouped=false/>
                        <button class="link" on:click=move |_| { c.pins_tab.set(false); ops::jump_to_message(c, id.clone(), ts, None); }>{tr().view_in_chat}</button>
                    </div>
                }.into_any(),
                Some((_, _, None)) => view! { <div class="loading-older"><div class="spinner small"></div></div> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn ChannelIntro() -> impl IntoView {
    let c = ctx();
    let info = Memo::new(move |_| {
        let id = c.current.get();
        c.snap.with(|s| {
            s.chat(&id)
                .map(|ch| (ch.clone(), display_title(s, ch), chat_face(s, ch)))
        })
    });
    move || {
        info.get().map(|(ch, title, face)| match (&ch.kind, face) {
            // Like "This is your space" in Slack.
            (ChatKind::Notes, Some(me)) => view! {
                <div class="intro notes">
                    <div class="intro-face">
                        <Avatar user=me.clone() size=120/>
                        <span class="intro-name">{tr().me_suffix(&me.display_name)}</span>
                    </div>
                    <p>
                        <b>{tr().intro_notes_bold}</b>
                        {tr().intro_notes_text}
                    </p>
                </div>
            }
            .into_any(),
            (ChatKind::Direct { .. }, Some(peer)) => view! {
                <div class="intro notes">
                    <div class="intro-face">
                        <Avatar user=peer.clone() size=120/>
                        <span class="intro-name">{peer.display_name.clone()}</span>
                    </div>
                    <p>{tr().intro_dm(&peer.display_name)}</p>
                </div>
            }
            .into_any(),
            _ => {
                let text = ch
                    .description
                    .clone()
                    .map(|d| tr().intro_channel_with(&d))
                    .unwrap_or_else(|| tr().intro_channel.to_owned());
                view! {
                    <div class="intro">
                        <div class="intro-title">
                            <span class="ch-kind">{kind_icon(&Mark::of(&ch), 26)}</span>
                            <span>{title}</span>
                        </div>
                        <p>{text}</p>
                    </div>
                }
                .into_any()
            }
        })
    }
}

/// What the lines of a feed share: the names that mentions show, and where the read line goes.
#[derive(Clone, Copy)]
struct FeedCtx {
    names: Memo<mostik_core::markup::MentionNames>,
    read_line: Option<Memo<Option<receipts::ReadLine>>>,
}

/// Mention names for a message line: those of its feed; a line outside a feed (a pinned message,
/// the parent of a thread) follows the snapshot itself.
fn feed_names(c: Ctx) -> Memo<mostik_core::markup::MentionNames> {
    use_context::<FeedCtx>()
        .map(|f| f.names)
        .unwrap_or_else(|| Memo::new(move |_| c.snap.with(Snapshot::mention_names)))
}

/// Calendar day of a feed: the chat, the day and the time of its first line.
type FeedDayKey = (ChatId, (u32, u32, u32), Timestamp);

/// A feed keyed by message. In a chat messages are grouped by day, and the day badge sticks only
/// within its group, the next day pushes it out, as in Slack; threads have no days. A line is
/// built once and follows its message: a new message, a reaction or an edit elsewhere keeps the
/// text selected, the open reaction picker with its search and the editor of a message.
#[component]
fn Feed(
    messages: Memo<Vec<Message>>,
    #[prop(optional)] in_thread: bool,
    #[prop(optional)] read_line: Option<Memo<Option<receipts::ReadLine>>>,
) -> impl IntoView {
    let c = ctx();
    provide_context(FeedCtx {
        names: Memo::new(move |_| c.snap.with(Snapshot::mention_names)),
        read_line,
    });
    let days = Memo::new(move |_| messages.with(|list| feed::layout(list, time::day_key)));
    let line = move |key: (ChatId, Timestamp)| view! { <FeedRow days=days ts=key.1 in_thread=in_thread/> };
    let key_of = |l: &feed::FeedLine| (l.msg.chat_id.clone(), l.msg.ts);
    if in_thread {
        let keys = Memo::new(move |_| days.with(|d| d.iter().flat_map(|d| &d.lines).map(key_of).collect::<Vec<_>>()));
        return view! { <For each=move || keys.get() key=|k| k.clone() children=line/> }.into_any();
    }
    let day_keys = Memo::new(move |_| {
        days.with(|d| {
            d.iter()
                .map(|d| (d.lines[0].msg.chat_id.clone(), d.day, d.lines[0].msg.ts))
                .collect::<Vec<FeedDayKey>>()
        })
    });
    view! {
        <For each=move || day_keys.get() key=|(chat, day, _)| (chat.clone(), *day) children=move |(chat, day, first)| {
            // The lines of this day of this chat: an outgoing chat's day must not take the new chat's lines.
            let keys = move || {
                days.with(|d| {
                    d.iter()
                        .find(|x| x.day == day)
                        .map(|x| x.lines.iter().filter(|l| l.msg.chat_id == chat).map(key_of).collect::<Vec<_>>())
                        .unwrap_or_default()
                })
            };
            view! {
                <div class="day-group">
                    <DayDivider ts=first/>
                    <For each=keys key=|k| k.clone() children=line/>
                </div>
            }
        }/>
    }
    .into_any()
}

/// A line of a [`Feed`]: it finds its message in the layout after every change, and only the
/// parts of it that changed are built again.
#[component]
fn FeedRow(days: Memo<Vec<feed::FeedDay<(u32, u32, u32)>>>, ts: Timestamp, in_thread: bool) -> impl IntoView {
    let Some(first) = days.with_untracked(|d| feed::find_line(d, ts).cloned()) else {
        return ().into_any();
    };
    if first.msg.system.is_some() {
        return view! { <SystemRow msg=first.msg/> }.into_any();
    }
    // A call line is small and changes when the call ends: it is simply built again.
    if first.msg.call.is_some() {
        let line = Memo::new(move |_| days.with(|d| feed::find_line(d, ts).cloned()));
        return (move || {
            line.get()
                .map(|l| view! { <CallRow msg=l.msg end=l.call_end live=l.live_call/> })
        })
        .into_any();
    }
    // A line on its way out keeps its last message.
    let msg = Memo::new(move |prev: Option<&Message>| {
        days.with(|d| feed::find_line(d, ts).map(|l| l.msg.clone()))
            .or_else(|| prev.cloned())
            .unwrap_or_else(|| first.msg.clone())
    });
    let grouped = Memo::new(move |_| days.with(|d| feed::find_line(d, ts).is_some_and(|l| l.grouped)));
    view! { <MessageRow msg=msg grouped=grouped in_thread=in_thread/> }.into_any()
}

/// How many people are in the call going on in the chat, as Telemost last reported.
fn in_call(c: Ctx, chat: &ChatId) -> Option<u32> {
    c.meetings.with(|m| {
        m.get(chat)
            .filter(|m| !m.ended && m.participants > 0)
            .map(|m| m.participants)
    })
}

/// What a call line says, and whether it is a missed call. `anchor` is the note the line stands
/// for (the start, or a lone end), `end` the note of its end, if it ended.
fn call_title(anchor: &CallNote, end: Option<&Message>, me: &UserId) -> (&'static str, bool) {
    let ended = end.and_then(|e| match &e.call {
        Some(CallNote::Ended { reason, .. }) => Some((*reason, &e.author)),
        _ => None,
    });
    match ended {
        None if matches!(anchor, CallNote::Ringing { .. }) => (tr().call_incoming, false),
        None => (tr().call_started, false),
        Some((Some(CallEndReason::Declined), _)) => (tr().call_declined, false),
        // The end note comes from the caller: their own call nobody answered, or one the user missed.
        Some((Some(r), author)) if r.unanswered() && author == me => (tr().call_no_answer, false),
        Some((Some(r), _)) if r.unanswered() => (tr().call_missed, true),
        Some((reason, _)) if reason.is_some() || matches!(anchor, CallNote::Ringing { .. }) => {
            (tr().call_direct, false)
        }
        Some(_) => (tr().call_done, false),
    }
}

/// A call in the feed, like Slack's huddle message: who started it and "Join" while it goes on;
/// once it ended, how it went (a missed or declined direct call) or how long it lasted.
#[component]
fn CallRow(msg: Message, end: Option<Message>, live: bool) -> impl IntoView {
    let c = ctx();
    let Some(anchor) = msg.call.clone() else {
        return ().into_any();
    };
    let me = c.snap.with_untracked(|s| s.me.clone());
    let (title, missed) = call_title(&anchor, end.as_ref(), &me);
    let direct = match (&anchor, end.as_ref().and_then(|e| e.call.as_ref())) {
        (CallNote::Ringing { .. }, _) => true,
        (_, Some(CallNote::Ended { reason, .. })) => reason.is_some(),
        _ => false,
    };
    let going = end.is_none();
    // A direct chat needs no name: it is the other person's call or the user's own.
    let who = (!direct && !matches!(anchor, CallNote::Ended { .. })).then(|| {
        let name = c
            .snap
            .with_untracked(|s| s.user(&msg.author).map(|u| u.display_name.clone()))
            .unwrap_or_else(|| tr().user.to_owned());
        let id = msg.author.clone();
        view! { <button class="msg-author" on:click=move |_| ops::open_profile(c, id.clone())>{name}</button> }
    });
    let unanswered = end
        .as_ref()
        .is_some_and(|e| matches!(&e.call, Some(CallNote::Ended { reason: Some(r), .. }) if r.unanswered()));
    let start = (!matches!(anchor, CallNote::Ended { .. })).then_some(msg.ts);
    let lasted = end
        .as_ref()
        .filter(|_| !unanswered)
        .and_then(|e| call_seconds(e, start))
        .map(|s| tr().call_lasted(s / 60));
    let chat = msg.chat_id.clone();
    // How many are in the call while it goes on, as Telemost last reported.
    let people = live.then(|| {
        let chat = chat.clone();
        view! {
            {move || in_call(c, &chat).map(|n| view! {
                <span class="call-note-sep">"·"</span><span class="call-note-meta">{tr().in_call(n)}</span>
            })}
        }
    });
    // The button stays while the call goes on: a later end note removes it.
    let join = live.then(|| {
        view! {
            {move || c.active_calls.with(|a| a.contains_key(&chat)).then(|| {
                let chat = chat.clone();
                view! {
                    <button class="btn small call-join" on:click=move |_| ops::start_call(c, chat.clone())>
                        <Icon icon=if direct { I::Phone } else { I::Headphones } size=14/><span>{tr().call_join}</span>
                    </button>
                }
            })}
        }
    });
    let icon = match (direct, missed || unanswered) {
        (true, true) => I::PhoneOff,
        (true, false) => I::Phone,
        _ => I::Headphones,
    };
    view! {
        <div class="msg call-note" class:ended=!going class:missed=missed data-ts=msg.ts.to_string()>
            <span class="call-note-icon"><span class="call-note-badge"><Icon icon=icon size=16/></span></span>
            <div class="call-note-body">
                <span class="call-note-title">{title}</span>
                {who.map(|w| view! { <span class="call-note-sep">"·"</span>{w} })}
                <span class="msg-time">{time::hm(msg.ts)}</span>
                {lasted.map(|l| view! { <span class="call-note-sep">"·"</span><span class="call-note-meta">{l}</span> })}
                {people}
            </div>
            {join}
        </div>
    }
    .into_any()
}

/// What a note about the chat says after the name of its author.
fn chat_note_text(s: &Snapshot, m: &Message) -> Option<String> {
    let note = m.system.as_ref()?;
    let name = |id: &UserId| {
        s.user(id)
            .filter(|u| u.has_name())
            .map_or_else(|| tr().someone.to_owned(), |u| u.display_name.clone())
    };
    Some(tr().chat_note(note, &m.author, name))
}

/// A note about the chat, like Slack's "joined #channel": who did what, and when.
#[component]
fn SystemRow(msg: Message) -> impl IntoView {
    let c = ctx();
    let Some(note) = msg.system.clone() else {
        return ().into_any();
    };
    let (name, text) = c.snap.with_untracked(|s| {
        (
            s.user(&msg.author)
                .map_or_else(|| tr().someone.to_owned(), |u| u.display_name.clone()),
            chat_note_text(s, &msg).unwrap_or_default(),
        )
    });
    let icon = match note {
        SystemNote::Created { .. } => I::Hash,
        SystemNote::Members { added, .. } if added.is_empty() => I::UserMinus,
        SystemNote::Left => I::UserMinus,
        SystemNote::Members { .. } | SystemNote::Joined => I::UserPlus,
    };
    let id = msg.author.clone();
    view! {
        <div class="msg call-note ended chat-note" data-ts=msg.ts.to_string()>
            <span class="call-note-icon"><span class="call-note-badge"><Icon icon=icon size=16/></span></span>
            <div class="call-note-body">
                <span class="chat-note-text">
                    <button class="msg-author" on:click=move |_| ops::open_profile(c, id.clone())>{name}</button>
                    " "{text}
                </span>
                <span class="msg-time">{time::hm(msg.ts)}</span>
            </div>
        </div>
    }
    .into_any()
}

#[component]
fn DayDivider(ts: Timestamp) -> impl IntoView {
    view! {
        <div class="day-divider">
            <span class="day-pill">{time::day_label(ts)}</span>
        </div>
    }
}

/// Threads are supported in group chats (`0/<ns>/<uuid>`).
fn threads_supported(chat: &ChatId) -> bool {
    chat.0.starts_with("0/")
}

/// Avatars under a message with a thread, as in Slack.
const MAX_THREAD_FACES: usize = 5;

/// Message text: paragraphs with inline markup and code blocks.
fn render_text(text: &str, who: &Mentionable, me: Me) -> AnyView {
    mostik_core::markup::parse_blocks(text)
        .into_iter()
        .map(|b| match b {
            mostik_core::markup::Block::Code(code) => view! { <pre class="code-block">{code}</pre> }.into_any(),
            mostik_core::markup::Block::Text(t) => render_inline(&t, who, me),
        })
        .collect_view()
        .into_any()
}

fn render_inline(text: &str, who: &Mentionable, me: Me) -> AnyView {
    parse_inline(text, who)
        .into_iter()
        .map(|p| match p {
            Inline::Text(t) => custom_text(t),
            Inline::Bold(t) => view! { <b>{t}</b> }.into_any(),
            Inline::Italic(t) => view! { <i>{t}</i> }.into_any(),
            Inline::Strike(t) => view! { <s>{t}</s> }.into_any(),
            Inline::Underline(t) => view! { <u>{t}</u> }.into_any(),
            Inline::Code(t) => view! { <code>{t}</code> }.into_any(),
            Inline::Link { text, url } => {
                let (target, middle) = (url.clone(), url.clone());
                // A middle click is no `click`: left to the webview, WebKitGTK opens the link
                // right in the app window. It opens in the default browser, like a click.
                view! {
                    <a class="msg-link" href=url.clone() title=url on:click=move |e| {
                        e.prevent_default();
                        ops::open_url(ctx(), target.clone());
                    } on:auxclick=move |e| {
                        e.prevent_default();
                        if e.button() == 1 {
                            ops::open_url(ctx(), middle.clone());
                        }
                    }>{text}</a>
                }
                .into_any()
            }
            Inline::Mention(n) => {
                let cls = if n == me.name { "mention me" } else { "mention" };
                view! { <span class=cls>"@"{n}</span> }.into_any()
            }
            Inline::UserMention { id, name } => {
                let cls = if id == me.id {
                    "mention me person"
                } else {
                    "mention person"
                };
                let name = name.unwrap_or_else(|| tr().unknown_user.to_owned());
                let profile = UserId(id);
                view! {
                    <span class=cls on:click=move |e| {
                        e.stop_propagation();
                        ops::open_profile(ctx(), profile.clone());
                    }>"@"{name}</span>
                }
                .into_any()
            }
            // The user is in every public chat that can be mentioned, and in the chat for `@all`.
            Inline::ChannelMention(n) => view! { <span class="mention me group">"@"{n}</span> }.into_any(),
            Inline::MentionAll => view! { <span class="mention me group">"@"{MENTION_ALL}</span> }.into_any(),
        })
        .collect_view()
        .into_any()
}

/// Plain text with the custom emoji of its `:name:` codes as images.
fn custom_text(t: String) -> AnyView {
    let set = crate::custom::current(ctx());
    let Some(set) = set.filter(|s| !s.emoji.is_empty() && t.contains(':')) else {
        return view! { <span>{t}</span> }.into_any();
    };
    mostik_core::custom_emoji::split(&t, |n| set.by_name(n).is_some())
        .into_iter()
        .map(|p| match p {
            mostik_core::custom_emoji::Piece::Text(x) => view! { <span>{x.to_owned()}</span> }.into_any(),
            mostik_core::custom_emoji::Piece::Emoji(n) => set
                .by_name(n)
                .map(|e| crate::custom::image(e, "custom-emoji").into_any())
                .unwrap_or_else(|| view! { <span>{format!(":{n}:")}</span> }.into_any()),
        })
        .collect_view()
        .into_any()
}

/// A message in the feed. Its parts follow `msg` each on its own, so a change rebuilds only what
/// changed: a new reaction keeps the text selected in the message and an open reaction picker.
#[component]
fn MessageRow(
    #[prop(into)] msg: Signal<Message>,
    #[prop(into)] grouped: Signal<bool>,
    #[prop(optional)] in_thread: bool,
) -> impl IntoView {
    let c = ctx();
    let names = feed_names(c);
    // What never changes in a message: where it is and who wrote it.
    let (chat_id, ts, author_id, thread_parent) =
        msg.with_untracked(|m| (m.chat_id.clone(), m.ts, m.author.clone(), m.thread_parent));
    let (me_name, me_id) = c.snap.with_untracked(|s| {
        (
            s.user(&s.me).map(|u| u.display_name.clone()).unwrap_or_default(),
            s.me.clone(),
        )
    });
    let mine = author_id == me_id;
    let key = (chat_id.clone(), ts);
    let author = {
        let id = author_id.clone();
        Memo::new(move |_| c.snap.with(|s| s.user(&id).cloned()))
    };
    let text = Memo::new(move |_| msg.with(|m| (m.text.clone(), m.edited)));
    let important = Memo::new(move |_| msg.with(|m| m.important));
    // A reply shows the original above its text; forwarded messages go below it.
    let replied = Memo::new(move |_| msg.with(|m| m.reply_to().cloned()));
    let forwarded = Memo::new(move |_| {
        msg.with(|m| match m.reply_to() {
            Some(_) => vec![],
            None => m.forwarded.clone(),
        })
    });
    let attachments = Memo::new(move |_| msg.with(|m| m.attachments.clone()));
    let reactions = Memo::new(move |_| msg.with(|m| m.reactions.clone()));
    let thread = Memo::new(move |_| msg.with(|m| m.thread.clone().filter(|t| !in_thread && t.reply_count > 0)));
    let mentions_me = {
        let (me_id, me_name) = (me_id.clone(), me_name.clone());
        Memo::new(move |_| {
            let me = Me {
                id: &me_id.0,
                name: &me_name,
            };
            !mine && text.with(|(t, _)| names.with(|n| n.with(|who| mostik_core::markup::mentions_me(t, who, me))))
        })
    };
    let editing = {
        let key = key.clone();
        Memo::new(move |_| {
            c.editing
                .with(|e| e.as_ref().is_some_and(|m| (m.chat_id.clone(), m.ts) == key))
        })
    };

    let open_thread = move |_| ops::open_thread(c, ts);
    let show_profile = {
        let id = author_id.clone();
        move |_| ops::open_profile(c, id.clone())
    };
    // A right click on the name or avatar opens the person menu, not the message menu.
    let user_ctx = move |e: ev::MouseEvent| {
        e.prevent_default();
        e.stop_propagation();
        c.menu.set(None);
        c.picker.set(None);
        c.ctx_menu.set(Some(CtxMenu {
            x: e.client_x() as f64,
            y: e.client_y() as f64,
            target: CtxTarget::User(author_id.clone()),
        }));
    };
    let gutter = {
        let (show_profile, user_ctx) = (show_profile.clone(), user_ctx.clone());
        move || {
            if grouped.get() {
                return view! { <span class="msg-hover-time">{time::hm(ts)}</span> }.into_any();
            }
            match author.get() {
                Some(u) => view! { <button class="face-btn" on:click=show_profile.clone() on:contextmenu=user_ctx.clone()><Avatar user=u size=36/></button> }.into_any(),
                None => view! { <span class="avatar unknown"></span> }.into_any(),
            }
        }
    };
    let header = move || {
        (!grouped.get()).then(|| {
            let author = author.get();
            let name = author
                .as_ref()
                .map_or_else(|| tr().user.to_owned(), |u| u.display_name.clone());
            let robot = author.as_ref().is_some_and(|u| u.is_robot);
            view! {
                <div class="msg-head">
                    <button class="msg-author" on:click=show_profile.clone() on:contextmenu=user_ctx.clone()>{name}</button>
                    {author.as_ref().and_then(status_badge)}
                    {robot.then(|| view! { <span class="app-tag">{tr().bot}</span> })}
                    <span class="msg-time">{time::hm(ts)}</span>
                </div>
            }
        })
    };
    let important_label = move || {
        important.get().then(|| {
            view! { <div class="important-label"><Icon icon=I::Bookmark size=12/><span>{tr().important}</span></div> }
        })
    };
    // Like "Pinned to this conversation" in Slack.
    let pinned = {
        let chat = chat_id.clone();
        Memo::new(move |_| {
            thread_parent.is_none() && c.snap.with(|s| s.chat(&chat).and_then(|ch| ch.pinned)) == Some(ts)
        })
    };
    let pinned_label = move || {
        pinned.get().then(|| {
            view! { <div class="important-label pinned-label"><Icon icon=I::Pin size=12/><span>{tr().pinned_label}</span></div> }
        })
    };
    let reply_block = {
        let chat = chat_id.clone();
        move || {
            replied
                .get()
                .map(|f| view! { <ReplyBlock f=f chat=chat.clone() thread_parent=thread_parent/> })
        }
    };
    let forwarded_blocks = move || {
        forwarded
            .get()
            .into_iter()
            .map(|f| view! { <ForwardedBlock f=f/> })
            .collect_view()
    };
    // The text is parsed again only when it changes, or the names it may mention do.
    let body = {
        let (me_id, me_name) = (me_id.clone(), me_name.clone());
        move || {
            text.with(|(t, edited)| {
                let me = Me {
                    id: &me_id.0,
                    name: &me_name,
                };
                let body = names.with(|n| n.with(|who| render_text(t, who, me)));
                let edited = edited.then(|| view! { <span class="edited">{tr().edited}</span> });
                (!t.is_empty() || edited.is_some()).then(|| view! { <div class="msg-text">{body}{edited}</div> })
            })
        }
    };
    let reactions_view = {
        let (key, chat_id) = (key.clone(), chat_id.clone());
        move || {
            let list = reactions.get();
            (!list.is_empty()).then(|| {
                let key = key.clone();
                view! {
                    <div class="reactions">
                        {list.into_iter().map(|r| {
                            let kind = r.kind;
                            let custom = crate::custom::by_kind(c, &chat_id, kind);
                            let title = c.snap.with_untracked(|s| reaction_title(s, &r, custom.as_ref()));
                            // Follows the set: it may arrive after the message, and an emoji may be renamed.
                            let (chat, glyph) = (chat_id.clone(), r.emoji.clone());
                            let glyph = move || match crate::custom::by_kind(c, &chat, kind) {
                                Some(e) => crate::custom::image(&e, "custom-emoji").into_any(),
                                None if mostik_core::custom_emoji::is_custom_reaction(kind) => {
                                    view! { <span class="emoji" title=tr().unknown_custom_emoji>"▫️"</span> }.into_any()
                                }
                                None => view! { <span class="emoji">{glyph.clone()}</span> }.into_any(),
                            };
                            view! {
                                <button class=if r.mine { "reaction mine" } else { "reaction" }
                                    title=title
                                    on:click=move |_| msg.with_untracked(|m| ops::toggle_reaction(c, m, kind))>
                                    {glyph}
                                    <span class="count">{r.count}</span>
                                </button>
                            }
                        }).collect_view()}
                        <button class="reaction add" title=tr().add_reaction on:click=move |e| { c.picker_at.set(crate::emoji::anchor_of(&e)); c.picker.set(Some(key.clone())) }>
                            <Icon icon=I::SmilePlus size=16/>
                        </button>
                    </div>
                }
            })
        }
    };
    let thread_view = {
        let chat = chat_id.clone();
        move || {
            thread.get().map(|t| {
                let faces = c.snap.with_untracked(|s| {
                    // Telemost does not say who replied: the ones seen replying, or the loaded replies.
                    let ids = if t.participants.is_empty() {
                        s.thread_participants(&chat, ts)
                    } else {
                        t.participants.clone()
                    };
                    ids.iter()
                        .filter_map(|id| s.user(id).cloned())
                        .take(MAX_THREAD_FACES)
                        .collect::<Vec<_>>()
                });
                view! {
                    <button class="thread-summary" on:click=open_thread>
                        <span class="thread-faces">
                            {faces.into_iter().map(|u| view! { <Avatar user=u size=24/> }).collect_view()}
                        </span>
                        <span class="thread-count">
                            {tr().replies(t.reply_count)}
                        </span>
                        <span class="thread-last">{tr().last_reply(&time::relative(t.last_reply_ts))}</span>
                        <span class="thread-open">{tr().open_thread}</span>
                        <Icon icon=I::ChevronRight size=14/>
                    </button>
                }
            })
        }
    };
    // The read line of the chat, under the user's own message it reaches.
    let read_view = use_context::<FeedCtx>()
        .and_then(|f| f.read_line)
        .filter(|_| mine && !in_thread)
        .map(|line| {
            let status = Memo::new(move |_| line.with(|l| l.as_ref().filter(|l| l.ts == ts).map(|l| l.status.clone())));
            move || status.get().map(|status| view! { <ReadMark status=status/> })
        });
    let reply_btn = (!in_thread && threads_supported(&chat_id)).then(|| {
        view! { <button title=tr().reply_in_thread on:click=open_thread><Icon icon=I::Threads size=16/></button> }
    });
    let picker_open = {
        let key = key.clone();
        Memo::new(move |_| c.picker.with(|p| p.as_ref() == Some(&key)))
    };
    let ctx_open = {
        let key = key.clone();
        Memo::new(move |_| c.ctx_menu.with(|m| m.as_ref().is_some_and(|m| m.is_message(&key))))
    };
    let highlighted = {
        let key = key.clone();
        Memo::new(move |_| c.highlight.with(|h| h.as_ref() == Some(&key)))
    };
    // A right click on a message opens the same menu as "More", but at the cursor, as in Slack.
    // Selected text of the message keeps this menu: it offers to quote or copy the selection.
    let on_ctx = move |e: ev::MouseEvent| {
        let quote = selected_in_message(ts);
        if editing.get_untracked() || (quote.is_none() && wants_native_menu(&e)) {
            return;
        }
        e.prevent_default();
        c.menu.set(None);
        c.picker.set(None);
        c.ctx_menu.set(Some(CtxMenu {
            x: e.client_x() as f64,
            y: e.client_y() as f64,
            target: CtxTarget::Message {
                msg: Box::new(msg.get_untracked()),
                mine,
                quote,
            },
        }));
    };

    view! {
        <div class=move || {
            let mut cls = String::from("msg");
            if grouped.get() { cls.push_str(" grouped"); }
            if mentions_me.get() { cls.push_str(" mention-me"); }
            if important.get() { cls.push_str(" important"); }
            if editing.get() { cls.push_str(" editing"); }
            if picker_open.get() || ctx_open.get() { cls.push_str(" active"); }
            if highlighted.get() { cls.push_str(" highlight"); }
            cls
        } data-ts=ts.to_string() on:contextmenu=on_ctx>
            {important_label}
            {pinned_label}
            <div class="msg-row">
                <div class="msg-gutter">{gutter}</div>
                <div class="msg-content">
                    {header}
                    {reply_block}
                    // The editor depends on `editing` only: changes of the message do not rebuild it.
                    {move || if editing.get() {
                        view! { <InlineEditor msg=msg.get_untracked()/> }.into_any()
                    } else {
                        body.clone().into_any()
                    }}
                    {forwarded_blocks}
                    {move || view! { <Attachments list=attachments.get()/> }}
                    {reactions_view}
                    {thread_view}
                    {read_view}
                </div>
            </div>
            <div class="msg-actions">
                <button title=tr().reaction on:click={
                    let key = key.clone();
                    move |e| { c.picker_at.set(crate::emoji::anchor_of(&e)); c.picker.set(Some(key.clone())) }
                }><Icon icon=I::SmilePlus size=16/></button>
                {reply_btn}
                <button title=move || if important.get() { tr().unmark_important } else { tr().mark_important }
                    on:click=move |_| msg.with_untracked(|m| ops::toggle_important(c, m))>
                    <Icon icon=I::Bookmark size=16/>
                </button>
                // "More" is the same menu as the right click, under the button; at the window edge it opens upwards.
                // Pressing it keeps the text selected in the message, for "Quote selection".
                <button title=tr().more on:mousedown=|e: ev::MouseEvent| e.prevent_default() on:click=move |e: ev::MouseEvent| {
                    c.picker.set(None);
                    let (x, y) = e
                        .current_target()
                        .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
                        .map_or((e.client_x() as f64, e.client_y() as f64), |el| {
                            let r = el.get_bounding_client_rect();
                            (r.right(), r.bottom() + 4.0)
                        });
                    let quote = selected_in_message(ts);
                    c.ctx_menu.set(Some(CtxMenu { x, y, target: CtxTarget::Message { msg: Box::new(msg.get_untracked()), mine, quote } }));
                }><Icon icon=I::Kebab size=16/></button>
            </div>
            <Show when=move || picker_open.get()>
                <ReactionPicker msg=msg/>
            </Show>
        </div>
    }
}

/// The read line under an own message, in the spirit of Slack: muted, with a double check in a
/// direct chat and the faces of the latest readers in a group; the names show on hover.
#[component]
fn ReadMark(status: receipts::ReadStatus) -> impl IntoView {
    let c = ctx();
    match status {
        receipts::ReadStatus::Seen { at, .. } => view! {
            <div class="read-line">
                <Icon icon=I::CheckCheck size=14/>
                <span>{tr().seen(at.map(time::relative).as_deref())}</span>
            </div>
        }
        .into_any(),
        receipts::ReadStatus::Read {
            count,
            recent,
            everyone,
        } => {
            let (faces, title) = c.snap.with_untracked(|s| {
                let faces: Vec<User> = recent.iter().filter_map(|id| s.user(id).cloned()).collect();
                (faces, readers(s, count, &recent))
            });
            let mark = if faces.is_empty() {
                view! { <Icon icon=I::CheckCheck size=14/> }.into_any()
            } else {
                view! {
                    <span class="read-faces">
                        {faces.into_iter().map(|u| view! { <Avatar user=u size=16/> }).collect_view()}
                    </span>
                }
                .into_any()
            };
            view! {
                <div class="read-line" title=title>
                    {mark}
                    <span>{tr().read_by(count, everyone)}</span>
                </div>
            }
            .into_any()
        }
    }
}

/// Who read a message, for the hover of the read line: "Anna, Boris, Vera and 2 more".
fn readers(s: &Snapshot, count: u32, recent: &[UserId]) -> String {
    let names: Vec<String> = recent
        .iter()
        .filter_map(|id| s.user(id).filter(|u| u.has_name()).map(|u| u.display_name.clone()))
        .collect();
    tr().readers(&names, count.saturating_sub(names.len() as u32))
}

/// What the message menu says about the reads of an own message, if anything.
fn read_info(c: Ctx, msg: &Message) -> Option<String> {
    if !c.settings.with_untracked(|s| s.show_read_receipts) {
        return None;
    }
    c.snap.with_untracked(|s| {
        // The loaded copy is the fresher one: counters change while the menu is not open.
        let m = s
            .messages
            .iter()
            .find(|m| m.chat_id == msg.chat_id && m.ts == msg.ts)
            .unwrap_or(msg);
        match receipts::read_status(s.chat(&msg.chat_id)?, &s.me, m)? {
            receipts::ReadStatus::Seen { seen: false, .. } => Some(tr().not_seen_yet.to_owned()),
            receipts::ReadStatus::Seen { at, .. } => Some(tr().seen(at.map(time::relative).as_deref())),
            receipts::ReadStatus::Read { count: 0, .. } => Some(tr().not_read_yet.to_owned()),
            receipts::ReadStatus::Read {
                count,
                recent,
                everyone,
            } => {
                let head = tr().read_by(count, everyone);
                let names = readers(s, count, &recent);
                Some(if names.is_empty() {
                    head
                } else {
                    format!("{head}: {names}")
                })
            }
        }
    })
}

/// Forwarded message, like a "shared message" in Slack: a bar on the left, author, text and origin.
/// Reaction tooltip: who reacted, as in Slack. Telemost names only the latest people, so the
/// rest are counted; with nobody known it falls back to the reaction's name.
fn reaction_title(s: &Snapshot, r: &Reaction, custom: Option<&mostik_core::custom_emoji::CustomEmoji>) -> String {
    let names: Vec<String> = r
        .users
        .iter()
        .filter(|u| **u != s.me)
        .filter_map(|u| s.user(u).filter(|u| u.has_name()).map(|u| u.display_name.clone()))
        .collect();
    let more = r
        .count
        .saturating_sub(u32::from(r.mine))
        .saturating_sub(names.len() as u32);
    if names.is_empty() && !r.mine {
        return match custom {
            Some(e) => mostik_core::custom_emoji::caption(&e.name),
            None => reactions::name(r.kind).unwrap_or("").to_owned(),
        };
    }
    tr().reacted(&names, more, r.mine)
}

#[component]
fn ForwardedBlock(f: Forwarded) -> impl IntoView {
    let c = ctx();
    let (author, names, me_name, me_id, chat) = c.snap.with_untracked(|s| {
        (
            s.user(&f.author).cloned(),
            s.mention_names(),
            s.user(&s.me).map(|u| u.display_name.clone()).unwrap_or_default(),
            s.me.clone(),
            f.chat_id
                .as_ref()
                .and_then(|id| s.chat(id))
                .map(|ch| (ch.id.clone(), display_title(s, ch), ch.kind.clone())),
        )
    });
    let name = author
        .as_ref()
        .map(|u| u.display_name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| f.author_name.clone());
    let face = author.unwrap_or(User {
        id: f.author.clone(),
        display_name: name.clone(),
        title: None,
        presence: Presence::Offline,
        status: None,
        availability: Default::default(),
        is_robot: false,
        avatar: None,
    });
    let ts = f.ts;
    let footer = chat.map(|(id, title, kind)| {
        let prefix = if matches!(kind, ChatKind::Group { .. } | ChatKind::Broadcast) {
            "#"
        } else {
            ""
        };
        view! {
            <div class="fwd-footer">
                <button class="link" on:click=move |_| ops::jump_to_message(c, id.clone(), ts, None)>
                    {tr().posted_in(&format!("{prefix}{title}"))}
                </button>
                <span>" | "{time::relative(ts)}</span>
            </div>
        }
    });
    view! {
        <div class="fwd">
            <div class="fwd-head"><Avatar user=face size=20/><b>{name}</b></div>
            {(!f.text.is_empty()).then(|| view! { <div class="msg-text">{names.with(|who| render_text(&f.text, who, Me { id: &me_id.0, name: &me_name }))}</div> })}
            <Attachments list=f.attachments.clone()/>
            {footer}
        </div>
    }
}

/// One line about the message a reply answers: the quoted fragment, else its text without markup,
/// else its first attachment; a deleted message has none of them.
fn reply_snippet(s: &Snapshot, quote: Option<&str>, text: &str, attachments: &[Attachment]) -> String {
    let line = match quote.filter(|q| !q.trim().is_empty()) {
        Some(q) => q.to_owned(),
        None if !text.trim().is_empty() => s.mention_names().with(|who| mostik_core::markup::plain(text, who)),
        None => match attachments.first() {
            Some(a) if a.kind == AttachmentKind::Image => format!("🖼 {}", a.name),
            Some(a) => format!("📄 {}", a.name),
            None => tr().reply_deleted.to_owned(),
        },
    };
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Author's name for a reply: the known person, else the name Telemost sent with the original.
fn reply_author(s: &Snapshot, id: &UserId, sent_name: &str) -> String {
    s.user(id)
        .map(|u| u.display_name.clone())
        .filter(|n| !n.is_empty())
        .or_else(|| Some(sent_name.to_owned()).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| tr().user.to_owned())
}

/// The message a reply answers, above the reply's text, as in Telemost: author and the quoted
/// fragment or the start of the text. A click scrolls the feed to the original.
#[component]
fn ReplyBlock(f: Forwarded, chat: ChatId, thread_parent: Option<Timestamp>) -> impl IntoView {
    let c = ctx();
    let (name, line) = c.snap.with_untracked(|s| {
        (
            reply_author(s, &f.author, &f.author_name),
            reply_snippet(s, f.quote.as_deref(), &f.text, &f.attachments),
        )
    });
    let ts = f.ts;
    let quoted = f.quote.is_some();
    view! {
        <button class=if quoted { "reply-ref quoted" } else { "reply-ref" } title=tr().go_to_original
            on:click=move |_| ops::jump_to_message(c, chat.clone(), ts, thread_parent)>
            <span class="reply-author">{name}</span>
            <span class="reply-line">{line}</span>
        </button>
    }
}

/// The bar above the field while the user replies: to whom, what, and × to cancel.
#[component]
fn ReplyBar(target: ops::ReplyTarget) -> impl IntoView {
    let c = ctx();
    let m = &target.msg;
    let (name, line) = c.snap.with_untracked(|s| {
        (
            reply_author(s, &m.author, ""),
            reply_snippet(s, target.quote.as_deref(), &m.text, &m.attachments),
        )
    });
    let label = if target.quote.is_some() {
        tr().quoting(&name)
    } else {
        tr().replying_to(&name)
    };
    view! {
        <div class="reply-bar">
            <Icon icon=I::Reply size=16/>
            <div class="reply-bar-body">
                <span class="reply-author">{label}</span>
                <span class="reply-line">{line}</span>
            </div>
            <button class="icon-btn" title=tr().cancel_reply on:click=move |_| c.replying.set(None)>
                <Icon icon=I::X size=14/>
            </button>
        </div>
    }
}

/// A window event listener that lives as long as the component that adds it.
pub(crate) fn scoped_window_listener<E: ev::EventDescriptor + 'static>(event: E, cb: impl Fn(E::EventType) + 'static)
where
    E::EventType: wasm_bindgen::JsCast,
{
    let handle = leptos::__reexports::send_wrapper::SendWrapper::new(window_event_listener(event, cb));
    on_cleanup(move || handle.take().remove());
}

fn human_size(bytes: u64) -> String {
    tr().size(bytes)
}

#[component]
fn Attachments(list: Vec<Attachment>) -> impl IntoView {
    let c = ctx();
    (!list.is_empty()).then(|| {
        let inline = c.settings.with_untracked(|s| s.inline_media);
        let (images, files): (Vec<_>, Vec<_>) = list
            .into_iter()
            .partition(|a| a.kind == AttachmentKind::Image && inline);
        let single = images.len() == 1;
        view! {
            <div class=if single { "images single" } else { "images" }>
                {images.into_iter().map(|a| {
                    // Original proportions, at most 360×240 (a single image) or 160×160 (a gallery).
                    let (max_w, max_h) = if single { (360.0, 240.0) } else { (160.0, 160.0) };
                    let (w, h) = if a.width > 0 && a.height > 0 {
                        let k = (max_w / a.width as f64).min(max_h / a.height as f64).min(1.0);
                        (a.width as f64 * k, a.height as f64 * k)
                    } else {
                        (max_w, max_h)
                    };
                    let src = if c.live { bridge::file_url(&a.file_id, Some("middle")) } else { String::new() };
                    let full = if c.live { bridge::file_url(&a.file_id, None) } else { String::new() };
                    let (real_w, real_h) = (a.width, a.height);
                    let img = NodeRef::<html::Img>::new();
                    // Telemost's "middle" preview of a very wide or tall image is a crop of its centre:
                    // then the original is shown, otherwise the box shows a magnified piece of it.
                    let check_preview = move |_| {
                        let Some(el) = img.get_untracked() else { return };
                        if real_w == 0 || real_h == 0 || el.natural_height() == 0 || !el.src().contains("size=") {
                            return;
                        }
                        let preview = f64::from(el.natural_width()) / f64::from(el.natural_height());
                        if (preview / (real_w as f64 / real_h as f64) - 1.0).abs() > 0.05 {
                            el.set_src(&full);
                        }
                    };
                    let shown = a.clone();
                    view! {
                        <button class="image" style=format!("width:{w:.0}px;height:{h:.0}px") title=a.name.clone()
                            on:click=move |_| c.lightbox.set(Some(shown.clone()))>
                            <img node_ref=img src=src alt=a.name.clone() loading="lazy" on:load=check_preview/>
                        </button>
                    }
                }).collect_view()}
            </div>
            {files.into_iter().map(|a| {
                if a.kind == AttachmentKind::Image {
                    // Images outside the feed: a link with the name, they open in the viewer.
                    let shown = a.clone();
                    return view! {
                        <button class="image-link" on:click=move |_| c.lightbox.set(Some(shown.clone()))>
                            {format!("🖼 {}", a.name)}
                        </button>
                    }.into_any();
                }
                let (id, name) = (a.file_id.clone(), a.name.clone());
                let download = move |_| {
                    let (id, name) = (id.clone(), name.clone());
                    spawn_local(async move {
                        match bridge::save_file(&id, &name).await {
                            Ok(path) => c.notice.set(Some(format!("{}: {path}", tr().saved_to))),
                            Err(e) => c.error.set(Some(format!("{}: {e}", tr().err_download))),
                        }
                    });
                };
                view! {
                    <button class="file-card" title=tr().download_and_open on:click=download>
                        <span class="file-icon"><Icon icon=I::Files size=22/></span>
                        <span class="file-meta">
                            <span class="file-name">{a.name.clone()}</span>
                            <span class="file-size">{human_size(a.size)}</span>
                        </span>
                    </button>
                }.into_any()
            }).collect_view()}
        }
    })
}

/// Reaction picker: the full list of Telemost reactions with search, like the emoji picker in Slack.
#[component]
fn ReactionPicker(msg: Signal<Message>) -> impl IntoView {
    let c = ctx();
    let chat = msg.with_untracked(|m| m.chat_id.clone());
    // The message as it is at the pick: its reactions may have changed while the picker was open.
    let on_pick = Callback::new(move |key: String| {
        c.picker.set(None);
        if let Ok(kind) = key.parse::<u64>() {
            msg.with_untracked(|m| ops::toggle_reaction(c, m, kind));
        }
    });
    view! {
        <crate::emoji::EmojiPicker source=crate::emoji::Source::Reactions anchor=c.picker_at.get_untracked()
            custom=untrack(|| crate::custom::for_chat(c, &chat)) can_add=c.live on_pick=on_pick on_close=Callback::new(move |_| c.picker.set(None))/>
    }
}

/// Message menu items shared by the "More" button and the right click, like "More actions" in Slack.
fn message_menu_items(c: Ctx, msg: Message, mine: bool, quote: Option<String>) -> impl IntoView {
    let (m_edit, m_del, m_unread, m_link, m_fwd, m_pin) = (
        msg.clone(),
        msg.clone(),
        msg.clone(),
        msg.clone(),
        msg.clone(),
        msg.clone(),
    );
    let in_thread = msg.thread_parent.is_some();
    let pinned = c.snap.with_untracked(|s| s.chat(&msg.chat_id).and_then(|ch| ch.pinned)) == Some(msg.ts);
    let copy = |text: String| {
        let _ = window().navigator().clipboard().write_text(&text);
    };
    // Who read an own message, as a note above the items.
    let reads = read_info(c, &msg).map(|text| {
        view! {
            <div class="menu-info"><Icon icon=I::CheckCheck size=16/><span>{text}</span></div>
            <div class="menu-sep"></div>
        }
    });
    // Replies go to messages, not to notes; as in Telemost, they lead the menu.
    let replies = (!msg.is_note()).then(|| {
        let (m_reply, m_quote) = (msg.clone(), msg.clone());
        let selected = quote.clone();
        view! {
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); ops::start_reply(c, m_reply.clone(), None); }>
                {menu_item(I::Reply, tr().reply)}
            </button>
            {selected.map(|q| {
                let copied = q.clone();
                view! {
                    <button class="menu-icon-item" on:click=move |_| { close_menus(c); ops::start_reply(c, m_quote.clone(), Some(q.clone())); }>
                        {menu_item(I::Quote, tr().quote_selection)}
                    </button>
                    <button class="menu-icon-item" on:click=move |_| { close_menus(c); copy(copied.clone()); }>
                        {menu_item(I::Copy, tr().copy_selection)}
                    </button>
                }
            })}
            <div class="menu-sep"></div>
        }
    });
    view! {
        {reads}
        {replies}
        // Telemost does not make your own message unread, so the item is only for others' messages.
        {(!in_thread && !mine).then(|| view! {
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); ops::mark_unread(c, &m_unread); }>
                {menu_item(I::MarkUnread, tr().mark_unread)}
            </button>
        })}
        <button class="menu-icon-item" on:click=move |_| { close_menus(c); copy(ops::message_link(&m_link)); c.notice.set(Some(tr().link_copied.into())); }>
            {menu_item(I::Link, tr().copy_link)}
        </button>
        <button class="menu-icon-item" on:click=move |_| {
            close_menus(c);
            copy(c.snap.with_untracked(|s| s.mention_names().with(|who| mostik_core::markup::ids_to_names(&msg.text, who).0)));
        }>{menu_item(I::Copy, tr().copy_text)}</button>
        <div class="menu-sep"></div>
        <button class="menu-icon-item" on:click=move |_| { close_menus(c); c.forwarding.set(Some(m_fwd.clone())); }>
            {menu_item(I::Forward, tr().forward_message)}
        </button>
        {(!in_thread).then(|| view! {
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); ops::pin_message(c, &m_pin, !pinned); }>
                {if pinned { menu_item(I::PinOff, tr().unpin_message) } else { menu_item(I::Pin, tr().pin_message) }}
            </button>
        })}
        {mine.then(|| view! {
            <div class="menu-sep"></div>
            <button class="menu-icon-item" on:click=move |_| {
                close_menus(c);
                c.editing.set(Some(m_edit.clone()));
            }>{menu_item(I::Pencil, tr().edit)}</button>
            <button class="menu-icon-item danger" on:click=move |_| {
                close_menus(c);
                c.confirm_delete.set(Some(m_del.clone()));
            }>{menu_item(I::Trash, tr().delete_message)}</button>
        })}
    }
}

/// A menu item with its icon, as in Slack's message menu.
fn menu_item(icon: I, label: &'static str) -> impl IntoView {
    view! { <Icon icon=icon size=16/><span>{label}</span> }
}

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
fn insert_text(area: &web_sys::HtmlElement, text: &str, drop_before: usize) -> String {
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
fn mention_query(area: &web_sys::HtmlElement) -> Option<String> {
    let caret = selection(area).0?;
    mention_query_before(&crate::editor::value(area), caret)
}

/// What the composer suggests while typing: an emoji after `:` or a mention after `@`.
#[derive(Clone)]
enum Suggestion {
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
fn mention_suggestions(s: &Snapshot, chat: &ChatId, query: &str, limit: usize) -> Vec<Suggestion> {
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
fn emoji_query(area: &web_sys::HtmlElement) -> Option<String> {
    let caret = selection(area).0?;
    emoji_code_before(&crate::editor::value(area), caret)
}

#[component]
fn Composer(#[prop(optional)] thread: bool) -> impl IntoView {
    let c = ctx();
    let (draft, set_draft) = signal(String::new());
    let sending = RwSignal::new(false);
    let area = NodeRef::<html::Div>::new();
    let parent = move || if thread { c.thread.get_untracked() } else { None };
    let emoji_at: RwSignal<Option<(f64, f64)>> = RwSignal::new(None);
    let suggestions: RwSignal<Vec<Suggestion>> = RwSignal::new(vec![]);
    let suggest_sel = RwSignal::new(0usize);

    // The field shows the draft: a restored draft, a sent message clearing it, formatting. Typing
    // already shows, so the field is filled again only when its text differs, or when the emoji set
    // arrives and turns codes into images. The caret stays where it was.
    Effect::new(move |_| {
        let text = draft.get();
        let lookup = crate::custom::lookup(c);
        let Some(el) = area.get() else { return };
        let shown = crate::editor::value(&el);
        if shown == text {
            crate::editor::tidy(&el, &lookup);
            return;
        }
        let focused = document().active_element().is_some_and(|a| a == **el);
        crate::editor::render(&el, &text, &lookup);
        if focused {
            crate::editor::set_caret(&el, utf16_len(&text));
        }
    });

    // Chat draft: fill it in on opening, save it while typing (main composer only). The draft keeps
    // only the text: a reply started in another chat ends when the chat changes.
    Effect::new(move |_| {
        let id = c.current.get();
        if !thread {
            set_draft.set(c.drafts.with_untracked(|d| d.get(&id).cloned().unwrap_or_default()));
            if c.replying
                .with_untracked(|r| r.as_ref().is_some_and(|r| r.msg.chat_id != id))
            {
                c.replying.set(None);
            }
        }
    });

    // The reply this field sends: to a message of its chat, in its thread for the thread field.
    let reply = Memo::new(move |_| {
        let (chat, place) = (c.current.get(), if thread { c.thread.get() } else { None });
        if thread && place.is_none() {
            return None;
        }
        c.replying.with(|r| {
            r.clone()
                .filter(|r| r.msg.chat_id == chat && r.msg.thread_parent == place)
        })
    });
    // Starting a reply puts the caret at the end of this field.
    Effect::new(move |prev: Option<Option<(ChatId, Timestamp)>>| {
        let now = reply.with(|r| r.as_ref().map(|r| (r.msg.chat_id.clone(), r.msg.ts)));
        if now.is_some()
            && prev.flatten() != now
            && let Some(el) = area.get_untracked()
        {
            let _ = el.focus();
            crate::editor::set_caret(&el, utf16_len(&crate::editor::value(&el)));
        }
        now
    });
    let remember = move |text: &str| {
        if thread {
            return;
        }
        let id = c.current.get_untracked();
        c.drafts.update(|d| {
            if text.trim().is_empty() {
                d.remove(&id);
            } else {
                d.insert(id, text.to_owned());
            }
        });
    };

    let placeholder = move || {
        if thread {
            return tr().reply_placeholder.to_owned();
        }
        let id = c.current.get();
        c.snap.with(|s| {
            s.chat(&id)
                .map(|ch| match ch.kind {
                    ChatKind::Direct { .. } => tr().message_to_person(&chat_title(s, ch)),
                    ChatKind::Notes => tr().note_placeholder.to_owned(),
                    _ => tr().message_to_channel(split_chat_icon(&ch.name).1),
                })
                .unwrap_or_default()
        })
    };
    let submit = move || {
        let text = draft.get_untracked().trim().to_owned();
        let with_files = !thread && !c.pending_files.with_untracked(Vec::is_empty);
        if (text.is_empty() && !with_files) || sending.get_untracked() {
            return;
        }
        let chat = c.current.get_untracked();
        let chat_for_draft = chat.clone();
        let thread_parent = parent();
        let sent_text = text.clone();
        let replying = reply.get_untracked();
        sending.set(true);
        spawn_local(async move {
            // A reply is a text message: attached files follow it without a caption. Files alone
            // keep the reply bar for the text that comes next.
            let replied = replying.is_some() && !text.is_empty();
            let result = match (replying.clone(), with_files) {
                (Some(r), true) if !text.is_empty() => {
                    match ops::send(c, chat.clone(), text, thread_parent, Some(r)).await {
                        Ok(()) => ops::send_files(c, chat, String::new(), thread_parent).await,
                        Err(e) => Err(e),
                    }
                }
                (_, true) => ops::send_files(c, chat, text, thread_parent).await,
                (r, false) => ops::send(c, chat, text, thread_parent, r).await,
            };
            // The composer may be gone by now (the user left the chat view while files uploaded).
            sending.try_set(false);
            if result.is_ok() && replied && c.replying.get_untracked() == replying {
                c.replying.set(None);
            }
            match result {
                Ok(()) => {
                    // Clear only what was sent: while the message was on its way the user may have
                    // switched to another chat or thread, or kept typing.
                    let unchanged = |d: &str| d.trim() == sent_text;
                    if !thread {
                        c.drafts.update(|d| {
                            if d.get(&chat_for_draft).is_some_and(|t| unchanged(t)) {
                                d.remove(&chat_for_draft);
                            }
                        });
                    }
                    let same_place = c.current.get_untracked() == chat_for_draft && parent() == thread_parent;
                    if same_place && draft.try_get_untracked().is_some_and(|d| unchanged(&d)) {
                        set_draft.set(String::new());
                    }
                }
                Err(e) => c.error.set(Some(format!("{}: {e}", tr().err_not_sent))),
            }
        });
    };
    // Picking a suggestion replaces the typed `:code` or `@name`.
    let accept_suggestion = move |i: usize| {
        let Some(pick) = suggestions.with_untracked(|s| s.get(i).cloned()) else {
            return;
        };
        if let Some(el) = area.get_untracked() {
            let text = match pick {
                Suggestion::Emoji { glyph, code } => {
                    crate::emoji::remember(crate::emoji::Source::Text, glyph);
                    insert_text(
                        &el,
                        glyph,
                        code.len().min(emoji_query(&el).map_or(0, |q| q.chars().count())) + 1,
                    )
                }
                Suggestion::Custom(e) => {
                    let code = mostik_core::custom_emoji::caption(&e.name);
                    crate::emoji::remember(crate::emoji::Source::Text, &code);
                    insert_text(&el, &code, emoji_query(&el).map_or(0, |q| q.chars().count()) + 1)
                }
                Suggestion::Person(_) | Suggestion::Channel(_) | Suggestion::All => {
                    let name = match pick {
                        Suggestion::Person(u) => {
                            c.picked.update(|p| {
                                p.insert(u.display_name.clone(), u.id);
                            });
                            u.display_name
                        }
                        Suggestion::Channel(n) => n,
                        _ => MENTION_ALL.to_owned(),
                    };
                    let typed = mention_query(&el).map_or(0, |q| q.chars().count() + 1);
                    insert_text(&el, &format!("@{name} "), typed)
                }
            };
            remember(&text);
            set_draft.set(text);
        }
        suggestions.set(vec![]);
    };
    let field = crate::compose::Field::new(area, move |text| {
        remember(&text);
        set_draft.set(text);
    });
    if !thread {
        crate::compose::attach_dropped(c);
    }
    let on_key = move |ev: web_sys::KeyboardEvent| {
        if ev.is_composing() || field.hotkey(&ev) {
            return;
        }
        let n = suggestions.with_untracked(Vec::len);
        if n > 0 {
            match ev.key().as_str() {
                "ArrowDown" => {
                    ev.prevent_default();
                    return suggest_sel.update(|i| *i = (*i + 1) % n);
                }
                "ArrowUp" => {
                    ev.prevent_default();
                    return suggest_sel.update(|i| *i = (*i + n - 1) % n);
                }
                "Enter" | "Tab" => {
                    ev.prevent_default();
                    return accept_suggestion(suggest_sel.get_untracked());
                }
                "Escape" => {
                    // Closes only the suggestions, not the reply bar.
                    ev.prevent_default();
                    ev.stop_propagation();
                    return suggestions.set(vec![]);
                }
                _ => {}
            }
        }
        match ev.key().as_str() {
            "Enter" if (enter_sends() && !ev.shift_key() && !ev.ctrl_key()) || (!enter_sends() && ev.ctrl_key()) => {
                ev.prevent_default();
                submit();
            }
            // A line break of our own: the browser would put the new line in a block.
            "Enter" => {
                ev.prevent_default();
                if let Some(el) = area.get_untracked() {
                    let text = insert_text(&el, "\n", 0);
                    remember(&text);
                    set_draft.set(text);
                }
            }
            // ↑ in an empty field edits your last message, as in Slack.
            "ArrowUp" if draft.with_untracked(String::is_empty) => {
                let chat = c.current.get_untracked();
                let tp = parent();
                let last = c.snap.with_untracked(|s| {
                    s.messages
                        .iter()
                        .filter(|m| m.chat_id == chat && m.author == s.me && m.thread_parent == tp && !m.is_note())
                        .max_by_key(|m| m.ts)
                        .cloned()
                });
                if let Some(m) = last {
                    ev.prevent_default();
                    c.editing.set(Some(m));
                }
            }
            _ => {}
        }
    };
    let typing_text = move || if thread { None } else { typing_line(c, &c.current.get()) };
    view! {
        <div class=if thread { "composer-wrap in-thread" } else { "composer-wrap" }>
            <div class="composer">
                {move || reply.get().map(|r| view! { <ReplyBar target=r/> })}
                {(!thread).then(|| view! {
                    <Show when=move || !c.pending_files.with(Vec::is_empty)>
                        <div class="pending-files">
                            {move || c.pending_files.get().into_iter().map(|f| {
                                let path = f.path.clone();
                                view! {
                                    <span class="pending-file">
                                        <span class="file-icon small">{if f.image { "🖼" } else { "📄" }}</span>
                                        <span class="file-meta">
                                            <span class="file-name">{f.name.clone()}</span>
                                            <span class="file-size">{human_size(f.size)}</span>
                                        </span>
                                        <button class="pf-remove" title=tr().remove_file
                                            on:click=move |_| c.pending_files.update(|p| p.retain(|x| x.path != path))>
                                            <Icon icon=I::X size=12/>
                                        </button>
                                    </span>
                                }
                            }).collect_view()}
                        </div>
                    </Show>
                })}
                <crate::compose::FormatBar field/>
                <div
                    class="editor"
                    node_ref=area
                    contenteditable="plaintext-only"
                    role="textbox"
                    aria-multiline="true"
                    data-placeholder=placeholder
                    on:input=move |_| {
                        let Some(text) = area.get_untracked().map(|el| crate::editor::tidy(&el, &crate::custom::lookup(c))) else { return };
                        remember(&text);
                        set_draft.set(text);
                        ops::typing(c, c.current.get_untracked());
                        let found = area.get_untracked().map(|el| {
                            if let Some(q) = emoji_query(&el) {
                                // Custom emoji first, as in Slack, leaving room for the standard ones.
                                let standard = crate::emoji::suggest(&q, 8);
                                let mut found: Vec<Suggestion> = crate::custom::suggest(c, &q, 8).into_iter().map(Suggestion::Custom).collect();
                                found.truncate(8usize.saturating_sub(standard.len()).max(4));
                                found.extend(standard.into_iter().map(|(glyph, code)| Suggestion::Emoji { glyph, code }));
                                found.truncate(8);
                                found
                            } else if let Some(q) = mention_query(&el) {
                                let chat = c.current.get_untracked();
                                c.snap.with_untracked(|s| mention_suggestions(s, &chat, &q, 8))
                            } else {
                                vec![]
                            }
                        }).unwrap_or_default();
                        suggest_sel.set(0);
                        suggestions.set(found);
                    }
                    on:keydown=on_key
                    on:paste=move |e: ev::ClipboardEvent| {
                        if thread || !crate::compose::attach_pasted(c, &e) {
                            field.paste_link(&e);
                        }
                    }
                    on:blur=move |_| set_timeout(move || { suggestions.try_set(vec![]); }, Duration::from_millis(150))
                ></div>
                <Show when=move || !suggestions.with(Vec::is_empty)>
                    <div class="emoji-suggest">
                        {move || suggestions.get().into_iter().enumerate().map(|(i, pick)| {
                            let row = match pick {
                                Suggestion::Emoji { glyph, code } => view! {
                                    <span class="glyph">{glyph}</span><span>{format!(":{code}:")}</span>
                                }.into_any(),
                                Suggestion::Custom(e) => view! {
                                    <span class="glyph">{crate::custom::image(&e, "custom-emoji")}</span>
                                    <span>{mostik_core::custom_emoji::caption(&e.name)}</span>
                                }.into_any(),
                                Suggestion::Person(u) => {
                                    let name = u.display_name.clone();
                                    let title = u.title.clone().unwrap_or_default();
                                    view! {
                                        <span class="glyph"><Avatar user=u size=20/></span>
                                        <span class="name">{name}</span><span class="hint">{title}</span>
                                    }.into_any()
                                }
                                Suggestion::Channel(n) => view! {
                                    <span class="glyph"><Icon icon=I::Hash size=16/></span>
                                    <span class="name">{n}</span><span class="hint">{tr().mention_channel_hint}</span>
                                }.into_any(),
                                Suggestion::All => view! {
                                    <span class="glyph">"@"</span>
                                    <span class="name">{MENTION_ALL}</span><span class="hint">{tr().mention_all_hint}</span>
                                }.into_any(),
                            };
                            view! {
                                <button class=move || if suggest_sel.get() == i { "active" } else { "" }
                                    on:mousedown=move |e| { e.prevent_default(); accept_suggestion(i); }>
                                    {row}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                </Show>
                <div class="composer-bottom">
                    {(!thread).then(|| view! {
                        <button class="cb round" title=tr().attach on:click=move |_| {
                            if !c.live {
                                return c.error.set(Some(tr().only_in_app_files.into()));
                            }
                            spawn_local(async move {
                                match bridge::pick_files().await {
                                    Ok(files) => c.pending_files.update(|p| {
                                        for f in files {
                                            if !p.iter().any(|x| x.path == f.path) { p.push(f); }
                                        }
                                    }),
                                    Err(e) => c.error.set(Some(e)),
                                }
                            });
                        }><Icon icon=I::Plus size=16/></button>
                    })}
                    <button class="cb" title=tr().emoji_btn on:click=move |e| emoji_at.set(Some(crate::emoji::anchor_of(&e)))>
                        <Icon icon=I::Smile size=18/>
                    </button>
                    {move || emoji_at.get().map(|at| view! {
                        <crate::emoji::EmojiPicker source=crate::emoji::Source::Text anchor=(at.0 + 324.0, at.1)
                            custom=crate::custom::for_chat(c, &c.current.get_untracked()) can_add=c.live
                            on_close=Callback::new(move |_| emoji_at.set(None))
                            on_pick=Callback::new(move |glyph: String| {
                                emoji_at.set(None);
                                if let Some(el) = area.get_untracked() {
                                    let text = insert_text(&el, &glyph, 0);
                                    remember(&text);
                                    set_draft.set(text);
                                }
                            })/>
                    })}
                    {move || sending.get().then(|| view! { <span class="composer-hint">{tr().uploading}</span> })}
                    <span class="composer-hint">{if crate::app::enter_sends() { tr().hint_shift_enter } else { tr().hint_ctrl_enter }}</span>
                    <div class="spacer"></div>
                    <div class=move || if (draft.with(|d| d.trim().is_empty()) && (thread || c.pending_files.with(Vec::is_empty))) || sending.get() { "send-group empty" } else { "send-group" }>
                        <button class="send" title=tr().send on:click=move |_| submit()><Icon icon=I::Send size=16/></button>
                    </div>
                </div>
            </div>
            <div class="typing">{typing_text}</div>
        </div>
    }
}

#[component]
fn ThreadPane() -> impl IntoView {
    let c = ctx();
    let open = Memo::new(move |_| Some((c.current.get(), c.thread.get()?)));
    let parent = Memo::new(move |_| {
        let (id, ts) = open.get()?;
        c.snap
            .with(|s| s.messages.iter().find(|m| m.chat_id == id && m.ts == ts).cloned())
    });
    let shown = Memo::new(move |_| parent.with(Option::is_some));
    let title = Memo::new(move |_| {
        let id = c.current.get();
        c.snap
            .with(|s| s.chat(&id).map(|ch| chat_title(s, ch)).unwrap_or_default())
    });
    let replies = Memo::new(move |_| {
        open.get()
            .map(|(id, parent)| c.snap.with(|s| s.thread_replies(&id, parent)))
            .unwrap_or_default()
    });
    view! {
        <aside class="thread-pane">
            <header class="tp-header">
                <div>
                    <div class="tp-title">{tr().thread}</div>
                    <div class="tp-sub">{move || shown.get().then(|| format!("# {}", title.get()))}</div>
                </div>
                <button class="icon-btn" title=tr().close on:click=move |_| c.thread.set(None)>
                    <Icon icon=I::X/>
                </button>
            </header>
            <div class="tp-scroll">
                // Built once per thread; the parent and the replies follow their changes.
                {move || {
                    open.track();
                    let first = shown.get().then(|| parent.get_untracked()).flatten()?;
                    let msg = Memo::new(move |prev: Option<&Message>| {
                        parent.get().or_else(|| prev.cloned()).unwrap_or_else(|| first.clone())
                    });
                    let n = move || replies.with(Vec::len) as u32;
                    Some(view! {
                        <MessageRow msg=msg grouped=false in_thread=true/>
                        // No "0 replies" once the last reply is deleted, as in Slack.
                        {move || (n() > 0).then(|| view! { <div class="tp-divider"><span>{tr().replies(n())}</span></div> })}
                        <Feed messages=replies in_thread=true/>
                    })
                }}
            </div>
            <Composer thread=true/>
        </aside>
    }
}

/// People search field with drop-down results; `on_pick` is called for the chosen one.
#[component]
fn PeoplePicker(
    #[prop(optional)] chat_id: Option<ChatId>,
    exclude: Signal<Vec<UserId>>,
    on_pick: Callback<UserProfile>,
) -> impl IntoView {
    let c = ctx();
    let query = RwSignal::new(String::new());
    let results: RwSignal<Vec<UserProfile>> = RwSignal::new(vec![]);
    let seq = RwSignal::new(0u32);
    // The field gets focus right away, as in Slack dialogs.
    let field = NodeRef::<html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = field.get() {
            let _ = el.focus();
        }
    });
    let search = move |q: String| {
        query.set(q.clone());
        if q.trim().chars().count() < 2 {
            results.set(vec![]);
            return;
        }
        if !c.live {
            // In the demo, search among the people of the loaded data.
            let ql = q.trim().to_lowercase();
            let local = c.snap.with_untracked(|s| {
                s.users
                    .iter()
                    .filter(|u| !u.is_robot && u.display_name.to_lowercase().contains(&ql))
                    .map(|u| UserProfile {
                        id: u.id.clone(),
                        display_name: u.display_name.clone(),
                        email: None,
                        position: u.title.clone(),
                        department: None,
                        organization: None,
                        nickname: None,
                        avatar: u.avatar.clone(),
                    })
                    .collect()
            });
            results.set(local);
            return;
        }
        // One request once typing pauses, not one per key; only the latest query answers.
        let n = seq.get_untracked() + 1;
        seq.set(n);
        let chat = chat_id.clone();
        // The dialog may close before the timer fires or the answer comes: its signals are gone then.
        set_timeout(
            move || {
                if seq.try_get_untracked() != Some(n) {
                    return;
                }
                spawn_local(async move {
                    let found = bridge::search_people(&q, chat.as_ref().map(|c| c.0.as_str())).await;
                    if seq.try_get_untracked() != Some(n) {
                        return;
                    }
                    match found {
                        Ok(list) => {
                            results.try_set(list);
                        }
                        Err(e) => c.error.set(Some(format!("{}: {e}", tr().err_search))),
                    }
                });
            },
            Duration::from_millis(250),
        );
    };
    view! {
        <div class="picker-field">
            <input class="text-input" node_ref=field placeholder=tr().search_people_ph prop:value=move || query.get()
                on:input=move |e| search(event_target_value(&e))/>
            {move || {
                let ex = exclude.get();
                let list: Vec<UserProfile> = results.get().into_iter().filter(|p| !ex.contains(&p.id)).collect();
                (!list.is_empty()).then(|| view! {
                    <div class="picker-results">
                        {list.into_iter().map(|p| {
                            let face = User { id: p.id.clone(), display_name: p.display_name.clone(), title: None, presence: Presence::Offline, status: None, availability: Default::default(), is_robot: false, avatar: p.avatar.clone() };
                            let picked = p.clone();
                            view! {
                                <button class="picker-row" on:click=move |_| { on_pick.run(picked.clone()); query.set(String::new()); results.set(vec![]); }>
                                    <Avatar user=face size=28/>
                                    <span class="dm-body">
                                        <span class="dm-name">{p.display_name.clone()}</span>
                                        <span class="dm-preview">{p.position.clone().or(p.email.clone()).unwrap_or_default()}</span>
                                    </span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                })
            }}
        </div>
    }
}

fn profile_face(p: &UserProfile) -> User {
    User {
        id: p.id.clone(),
        display_name: p.display_name.clone(),
        title: p.position.clone(),
        presence: Presence::Offline,
        status: None,
        availability: Default::default(),
        is_robot: false,
        avatar: p.avatar.clone(),
    }
}

/// "Browse channels", like in Slack: your channels and recommended public groups.
#[component]
fn BrowseChannelsDialog() -> impl IntoView {
    let c = ctx();
    let close = move || c.browsing.set(false);
    let query = RwSignal::new(String::new());
    let recommended: RwSignal<Vec<Chat>> = RwSignal::new(vec![]);
    let loading = RwSignal::new(c.live);
    let field = NodeRef::<html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = field.get() {
            let _ = el.focus();
        }
    });
    if c.live {
        spawn_local(async move {
            match bridge::recommended_chats().await {
                Ok(list) => {
                    recommended.try_set(list);
                }
                Err(e) => leptos::logging::warn!("recommended chats: {e}"),
            }
            loading.try_set(false);
        });
    }
    let matches = move |name: &str, desc: Option<&str>| {
        let q = query.get().trim().to_lowercase();
        q.is_empty() || name.to_lowercase().contains(&q) || desc.is_some_and(|d| d.to_lowercase().contains(&q))
    };
    let mine = move || {
        c.snap.with(|s| {
            let mut list: Vec<Chat> = s
                .chats
                .iter()
                .filter(|ch| matches!(ch.kind, ChatKind::Group { .. } | ChatKind::Broadcast) && in_current_org(c, ch))
                .filter(|ch| matches(&ch.name, ch.description.as_deref()))
                .cloned()
                .collect();
            list.sort_by_key(|ch| ch.name.to_lowercase());
            list
        })
    };
    let others = move || {
        let known: Vec<ChatId> = c.snap.with(|s| s.chats.iter().map(|ch| ch.id.clone()).collect());
        recommended
            .get()
            .into_iter()
            .filter(|ch| !known.contains(&ch.id) && matches(&ch.name, ch.description.as_deref()))
            .collect::<Vec<_>>()
    };
    let row = move |ch: Chat, joined: bool| {
        let id = ch.id.clone();
        let busy = RwSignal::new(false);
        let open = move |_| {
            if joined {
                close();
                return ops::open_chat(c, id.clone());
            }
            busy.set(true);
            let id = id.clone();
            spawn_local(async move {
                match bridge::join_chat(&id.0).await {
                    Ok(chat) => {
                        let cid = chat.id.clone();
                        c.snap.update(|s| {
                            if s.chat(&cid).is_none() {
                                s.chats.push(chat);
                                if let Some(sec) = s.sections.iter_mut().find(|sec| sec.id == "channels") {
                                    sec.chats.insert(0, cid.clone());
                                }
                            }
                        });
                        close();
                        ops::open_chat(c, cid);
                    }
                    Err(e) => c.error.set(Some(format!("{}: {e}", tr().err_join))),
                }
                busy.try_set(false);
            });
        };
        view! {
            <div class="browse-row">
                <span class="browse-icon">{kind_icon(&Mark::of(&ch), 16)}</span>
                <span class="dm-body">
                    <span class="dm-name">{split_chat_icon(&ch.name).1.to_owned()}</span>
                    <span class="dm-preview">
                        {joined.then(|| view! { <span class="joined">"✓ "{tr().joined}" · "</span> })}
                        {tr().members_count(ch.member_count)}
                        {ch.description.clone().map(|d| format!(" · {d}"))}
                    </span>
                </span>
                <button class=if joined { "btn small" } else { "btn small primary" } disabled=move || busy.get() on:click=open>
                    {if joined { tr().open_btn } else { tr().join_btn }}
                </button>
            </div>
        }
    };
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal browse-modal" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{tr().browse_channels}</h2>
                    <button class="btn small" on:click=move |_| { close(); c.creating_channel.set(Some(NewChannel::default())); }>{tr().create_channel}</button>
                    <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
                </header>
                <input class="text-input" node_ref=field placeholder=tr().search_channels_ph prop:value=move || query.get()
                    on:input=move |e| query.set(event_target_value(&e))/>
                <div class="browse-list">
                    {move || {
                        let rec = others();
                        (!rec.is_empty()).then(|| view! {
                            <div class="sr-title">{tr().recommended}</div>
                            {rec.into_iter().map(|ch| row(ch, false)).collect_view()}
                        })
                    }}
                    {move || loading.get().then(|| view! { <div class="sr-empty">{tr().searching}</div> })}
                    <div class="sr-title">{tr().your_channels}</div>
                    {move || {
                        let list = mine();
                        if list.is_empty() {
                            return view! { <div class="sr-empty">{tr().nothing_found}</div> }.into_any();
                        }
                        list.into_iter().map(|ch| row(ch, true)).collect_view().into_any()
                    }}
                </div>
            </div>
        </div>
    }
}

/// "Forward message", like Forward in Slack: where to, a comment and a preview.
#[component]
fn ForwardDialog(msg: Message) -> impl IntoView {
    let c = ctx();
    let close = move || c.forwarding.set(None);
    let query = RwSignal::new(String::new());
    let target: RwSignal<Option<ChatId>> = RwSignal::new(None);
    let text = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let field = NodeRef::<html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = field.get() {
            let _ = el.focus();
        }
    });
    let options = Memo::new(move |_| {
        let q = query.get().trim().to_lowercase();
        c.snap.with(|s| {
            let mut list: Vec<&Chat> = s
                .chats
                .iter()
                .filter(|ch| q.is_empty() || display_title(s, ch).to_lowercase().contains(&q))
                .collect();
            list.sort_by_key(|ch| std::cmp::Reverse(ch.last_ts));
            list.into_iter()
                .take(8)
                .map(|ch| (ch.id.clone(), display_title(s, ch), Mark::of(ch), chat_face(s, ch)))
                .collect::<Vec<_>>()
        })
    });
    let preview = {
        let author_name = c
            .snap
            .with_untracked(|s| s.user(&msg.author).map(|u| u.display_name.clone()))
            .unwrap_or_default();
        // A reply is forwarded as itself, not as the message it answers.
        let own = Forwarded {
            author: msg.author.clone(),
            author_name,
            ts: msg.ts,
            chat_id: Some(msg.chat_id.clone()),
            text: msg.text.clone(),
            attachments: msg.attachments.clone(),
            quote: None,
            reply: false,
        };
        match msg.forwarded.first() {
            Some(f) if msg.reply_to().is_none() => f.clone(),
            _ => own,
        }
    };
    let send = move || {
        let Some(to) = target.get_untracked() else { return };
        busy.set(true);
        let (m, text) = (msg.clone(), text.get_untracked());
        spawn_local(async move {
            match ops::forward(c, m, to, text).await {
                Ok(()) => {
                    c.notice.set(Some(tr().forwarded_notice.into()));
                    close();
                }
                Err(e) => c.error.set(Some(format!("{}: {e}", tr().err_forward))),
            }
            busy.try_set(false);
        });
    };
    let target_title = move || {
        target
            .get()
            .and_then(|id| c.snap.with(|s| s.chat(&id).map(|ch| display_title(s, ch))))
    };
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal create-channel forward-modal" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{tr().forward_message}</h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
                </header>
                {move || match target_title() {
                    Some(title) => view! {
                        <div class="chips"><span class="chip">{title}
                            <button on:click=move |_| target.set(None)><Icon icon=I::X size=12/></button>
                        </span></div>
                    }.into_any(),
                    None => view! {
                        <div class="picker-field">
                            <input class="text-input" node_ref=field placeholder=tr().forward_to_ph prop:value=move || query.get()
                                on:input=move |e| query.set(event_target_value(&e))/>
                            <div class="picker-results inline">
                                {move || options.get().into_iter().map(|(id, title, kind, face)| view! {
                                    <button class="picker-row" on:click=move |_| { target.set(Some(id.clone())); query.set(String::new()); }>
                                        {match face {
                                            Some(u) => view! { <Avatar user=u size=24/> }.into_any(),
                                            None => view! { <span class="sr-icon">{kind_icon(&kind, 16)}</span> }.into_any(),
                                        }}
                                        <span class="dm-name">{title}</span>
                                    </button>
                                }).collect_view()}
                            </div>
                        </div>
                    }.into_any(),
                }}
                <textarea class="text-input area" placeholder=tr().forward_comment_ph prop:value=move || text.get()
                    on:input=move |e| text.set(event_target_value(&e))></textarea>
                <ForwardedBlock f=preview/>
                <div class="modal-actions">
                    <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                    <button class="btn primary" disabled=move || target.get().is_none() || busy.get() on:click=move |_| send()>{tr().forward_btn}</button>
                </div>
            </div>
        </div>
    }
}

/// "New message" (the pencil in the sidebar header): pick a person to open or create a direct chat.
#[component]
fn NewMessageDialog() -> impl IntoView {
    let c = ctx();
    let close = move || c.new_message.set(false);
    let me = c.snap.with_untracked(|s| s.me.clone());
    let exclude = Signal::derive(move || vec![me.clone()]);
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal create-channel" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{tr().new_message}</h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
                </header>
                <label class="field-label">{tr().new_message_to}</label>
                <PeoplePicker exclude=exclude on_pick=Callback::new(move |p: UserProfile| {
                    close();
                    c.view.set(ViewMode::Home);
                    c.snap.update(|s| if s.user(&p.id).is_none() { s.users.push(profile_face(&p)); });
                    ops::message_user(c, p.id);
                })/>
            </div>
        </div>
    }
}

/// "Create a channel", as in Slack: step 1 is the name and description, step 2 is the people.
#[component]
fn CreateChannelDialog(to: NewChannel) -> impl IntoView {
    let c = ctx();
    let step = RwSignal::new(1u8);
    let name = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let public = RwSignal::new(false);
    let people: RwSignal<Vec<UserProfile>> = RwSignal::new(vec![]);
    let busy = RwSignal::new(false);
    // A stored value keeps the closures below `Copy`.
    let folder = StoredValue::new(to.folder);
    let close = move || c.creating_channel.set(None);
    let name_field = NodeRef::<html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = name_field.get() {
            let _ = el.focus();
        }
    });
    let create = move || {
        if !c.live {
            return c.error.set(Some(tr().in_app_only_groups.into()));
        }
        busy.set(true);
        let (n, d) = (name.get_untracked().trim().to_owned(), description.get_untracked());
        let members: Vec<String> = people.get_untracked().into_iter().map(|p| p.id.0).collect();
        let (public, folder) = (public.get_untracked(), folder.get_value());
        spawn_local(async move {
            match bridge::create_channel(&n, &d, members, public).await {
                Ok(chat) => {
                    let id = chat.id.clone();
                    // The push about the new chat may have added it already.
                    c.snap.update(|s| {
                        s.chats.retain(|ch| ch.id != id);
                        s.chats.push(chat);
                        for sec in &mut s.sections {
                            sec.chats.retain(|x| x != &id);
                        }
                        if let Some(sec) = s.sections.iter_mut().find(|sec| sec.id == "channels") {
                            sec.chats.insert(0, id.clone());
                        }
                    });
                    if let Some(folder) = folder {
                        ops::change_folders(c, |f| folders::move_chat(f, &id, Some(&folder), None));
                    }
                    close();
                    c.view.set(ViewMode::Home);
                    ops::open_chat(c, id);
                }
                Err(e) => c.error.set(Some(format!("{}: {e}", tr().err_create_channel))),
            }
            busy.try_set(false);
        });
    };
    let exclude = Signal::derive(move || {
        let mut v: Vec<UserId> = people.with(|p| p.iter().map(|x| x.id.clone()).collect());
        v.push(c.snap.with(|s| s.me.clone()));
        v
    });
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal create-channel" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{move || if step.get() == 1 { tr().create_channel.to_owned() } else { format!("{} #{}", tr().add_people_to, name.get()) }}</h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
                </header>
                {move || if step.get() == 1 {
                    view! {
                        <label class="field-label">{tr().channel_name}</label>
                        <div class="name-input"><span>"#"</span>
                            <input class="text-input bare" node_ref=name_field placeholder=tr().channel_name_ph maxlength="80" prop:value=move || name.get()
                                on:input=move |e| name.set(event_target_value(&e))/>
                        </div>
                        <label class="field-label">{tr().channel_desc}</label>
                        <textarea class="text-input area" placeholder=tr().channel_desc_ph prop:value=move || description.get()
                            on:input=move |e| description.set(event_target_value(&e))></textarea>
                        // "Visibility", as when creating a channel in Slack.
                        <label class="field-label">{tr().visibility}</label>
                        <label class="pref-radio">
                            <input type="radio" name="visibility" prop:checked=move || !public.get() on:change=move |_| public.set(false)/>
                            <span><b>{tr().vis_private}</b>" — "{tr().vis_private_hint}</span>
                        </label>
                        <label class="pref-radio">
                            <input type="radio" name="visibility" prop:checked=move || public.get() on:change=move |_| public.set(true)/>
                            <span><b>{tr().vis_public}</b>" — "{tr().vis_public_hint}</span>
                        </label>
                        <div class="modal-actions">
                            <button class="btn primary" disabled=move || name.with(|n| n.trim().is_empty()) on:click=move |_| step.set(2)>{tr().next}</button>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <PeoplePicker exclude=exclude on_pick=Callback::new(move |p: UserProfile| people.update(|v| v.push(p)))/>
                        <div class="chips">
                            {move || people.get().into_iter().map(|p| {
                                let id = p.id.clone();
                                view! {
                                    <span class="chip"><Avatar user=profile_face(&p) size=20/>{p.display_name.clone()}
                                        <button on:click=move |_| people.update(|v| v.retain(|x| x.id != id))><Icon icon=I::X size=12/></button>
                                    </span>
                                }
                            }).collect_view()}
                        </div>
                        <div class="modal-actions">
                            <button class="btn" on:click=move |_| step.set(1)>{tr().back}</button>
                            <button class="btn primary" disabled=move || busy.get() on:click=move |_| create()>
                                {move || if people.with(Vec::is_empty) { tr().skip_for_now } else { tr().create }}
                            </button>
                        </div>
                    }.into_any()
                }}
            </div>
        </div>
    }
}

/// Channel details, as in Slack: "About" (name, description, leave) and "Members".
#[component]
fn ChannelDetailsDialog(initial: DetailsTab) -> impl IntoView {
    let c = ctx();
    let tab = RwSignal::new(initial);
    let chat_id = c.current.get_untracked();
    let details: RwSignal<Option<bridge::ChatDetails>> = RwSignal::new(None);
    let members: RwSignal<Vec<ChatMember>> = RwSignal::new(vec![]);
    let name = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let confirm_leave = RwSignal::new(false);
    let member_query = RwSignal::new(String::new());
    let adding = RwSignal::new(false);
    let kind = c.snap.with_untracked(|s| s.chat(&chat_id).map(|ch| ch.kind.clone()));
    let close = move || c.details.set(None);
    let load = {
        let chat_id = chat_id.clone();
        move || {
            if !c.live {
                return;
            }
            let id = chat_id.clone();
            spawn_local(async move {
                // The dialog may be closed by the time the answers come: its signals are gone then.
                if let Ok(d) = bridge::chat_details(&id.0).await {
                    name.try_set(d.name.clone());
                    description.try_set(d.description.clone());
                    details.try_set(Some(d));
                }
                match bridge::chat_members(&id.0).await {
                    Ok(m) => {
                        let count = m.len() as u32;
                        members.try_set(m);
                        c.snap.update(|s| {
                            if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == id) {
                                ch.member_count = count;
                            }
                        });
                    }
                    Err(e) => c.error.set(Some(e)),
                }
            });
        }
    };
    load();
    if !c.live {
        c.snap.with_untracked(|s| {
            if let Some(ch) = s.chat(&chat_id) {
                name.set(ch.name.clone());
                description.set(ch.description.clone().unwrap_or_default());
            }
        });
    }
    let admin = Memo::new(move |_| !c.live || details.with(|d| d.as_ref().is_some_and(|d| d.role == "admin")));
    let save = {
        let id = chat_id.clone();
        move || {
            let (id, n, d) = (
                id.clone(),
                name.get_untracked().trim().to_owned(),
                description.get_untracked(),
            );
            if n.is_empty() {
                return;
            }
            spawn_local(async move {
                // Show the name and description as the server saved them.
                let (n, d) = if c.live {
                    match bridge::update_chat(&id.0, &n, &d).await {
                        Ok(saved) => (saved.name, saved.description),
                        Err(e) => return c.error.set(Some(format!("{}: {e}", tr().err_update_chat))),
                    }
                } else {
                    (n, d)
                };
                c.snap.update(|s| {
                    if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == id) {
                        ch.name = n.clone();
                        ch.description = (!d.trim().is_empty()).then(|| d.clone());
                    }
                });
                close();
            });
        }
    };
    let leave = {
        let id = chat_id.clone();
        move || {
            close();
            ops::leave_chat(c, id.clone());
        }
    };
    let change = {
        let id = chat_id.clone();
        let load = load.clone();
        move |guids: Vec<UserId>, add: bool| {
            if !c.live {
                return c.error.set(Some(tr().in_app_only_groups.into()));
            }
            let (id, load) = (id.clone(), load.clone());
            spawn_local(async move {
                match bridge::change_members(&id.0, guids.into_iter().map(|g| g.0).collect(), add).await {
                    Ok(()) => load(),
                    Err(e) => c.error.set(Some(format!("{}: {e}", tr().err_members))),
                }
            });
        }
    };
    let me = c.snap.with_untracked(|s| s.me.clone());
    let picker_chat = chat_id.clone();
    let exclude = Signal::derive(move || members.with(|m| m.iter().map(|x| x.profile.id.clone()).collect()));
    let title = move || name.with(|n| split_chat_icon(n).1.to_owned());
    let (save2, leave2, change_add, change_rm) = (save.clone(), leave.clone(), change.clone(), change);
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal details" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2 class="details-title">
                        {move || kind.clone().map(|kind| {
                            let emoji = name.with(|n| split_chat_icon(n).0.map(str::to_owned));
                            kind_icon(&Mark { kind, emoji }, 20)
                        })}
                        <span>{title}</span>
                    </h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
                </header>
                <nav class="ch-tabs modal-tabs">
                    <button class=move || if tab.get() == DetailsTab::About { "tab active" } else { "tab" } on:click=move |_| tab.set(DetailsTab::About)>{tr().about_channel}</button>
                    <button class=move || if tab.get() == DetailsTab::Members { "tab active" } else { "tab" } on:click=move |_| tab.set(DetailsTab::Members)>
                        {move || format!("{} {}", tr().members_tab, members.with(Vec::len))}
                    </button>
                </nav>
                <div class="details-body">
                    {move || match tab.get() {
                        DetailsTab::About => {
                            let (save, leave) = (save2.clone(), leave2.clone());
                            view! {
                                <div class="detail-card">
                                    <label class="field-label">{tr().channel_name}</label>
                                    <input class="text-input" prop:value=move || name.get() disabled=move || !admin.get()
                                        on:input=move |e| name.set(event_target_value(&e))/>
                                    <label class="field-label">{tr().channel_description}</label>
                                    <textarea class="text-input area" placeholder=tr().no_description prop:value=move || description.get() disabled=move || !admin.get()
                                        on:input=move |e| description.set(event_target_value(&e))></textarea>
                                    {move || (!admin.get()).then(|| view! { <p class="pref-hint flush">{tr().only_admins_edit}</p> })}
                                    <Show when=move || admin.get()>
                                        <div class="modal-actions"><button class="btn primary" on:click={let save = save.clone(); move |_| save()}>{tr().save}</button></div>
                                    </Show>
                                </div>
                                <div class="detail-card">
                                    {move || if confirm_leave.get() {
                                        let leave = leave.clone();
                                        view! {
                                            <p>{tr().leave_confirm}</p>
                                            <div class="modal-actions">
                                                <button class="btn" on:click=move |_| confirm_leave.set(false)>{tr().cancel}</button>
                                                <button class="btn danger" on:click=move |_| leave()>{tr().leave_channel}</button>
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! { <button class="link danger-text" on:click=move |_| confirm_leave.set(true)>{tr().leave_channel}</button> }.into_any()
                                    }}
                                </div>
                            }.into_any()
                        }
                        DetailsTab::Members => {
                            let (rm, me) = (change_rm.clone(), me.clone());
                            view! {
                                <input class="text-input member-search" placeholder=tr().find_members prop:value=move || member_query.get()
                                    on:input=move |e| member_query.set(event_target_value(&e))/>
                                <Show when=move || admin.get()>
                                    <button class="member-row add-people-row" on:click=move |_| adding.set(true)>
                                        <span class="pf-icon"><Icon icon=I::Plus size=18/></span>
                                        <span class="dm-name">{tr().add_people}</span>
                                    </button>
                                </Show>
                                <div class="member-list">
                                    {move || {
                                        let q = member_query.get().trim().to_lowercase();
                                        members.get().into_iter().filter(move |m| q.is_empty() || m.profile.display_name.to_lowercase().contains(&q)).map(|m| {
                                        let id = m.profile.id.clone();
                                        let removable = admin.get() && id != me;
                                        let (rm, pid) = (rm.clone(), id.clone());
                                        view! {
                                            <div class="member-row">
                                                <button class="face-btn" on:click=move |_| { close(); ops::open_profile(c, pid.clone()); }>
                                                    <Avatar user=profile_face(&m.profile) size=36/>
                                                </button>
                                                <span class="dm-body">
                                                    <span class="dm-name">{m.profile.display_name.clone()}{(m.role == "admin").then(|| view! { <span class="role-tag">{tr().admin}</span> })}</span>
                                                    <span class="dm-preview">{m.profile.position.clone().unwrap_or_default()}</span>
                                                </span>
                                                {removable.then(|| view! {
                                                    <button class="icon-btn" title=tr().remove_from_channel on:click=move |_| rm(vec![id.clone()], false)><Icon icon=I::X size=16/></button>
                                                })}
                                            </div>
                                        }
                                    }).collect_view()}}
                                </div>
                            }.into_any()
                        }
                    }}
                </div>
            </div>
            {move || adding.get().then(|| {
                let add = change_add.clone();
                view! {
                    <AddPeopleDialog chat_id=picker_chat.clone() title=name.get_untracked() exclude=exclude
                        on_close=Callback::new(move |_| adding.set(false))
                        on_add=Callback::new(move |ids: Vec<UserId>| { adding.set(false); add(ids, true); })/>
                }
            })}
        </div>
    }
}

/// "Add people to #channel", as in Slack: a separate dialog on top of the channel details.
#[component]
fn AddPeopleDialog(
    chat_id: ChatId,
    title: String,
    exclude: Signal<Vec<UserId>>,
    on_close: Callback<()>,
    on_add: Callback<Vec<UserId>>,
) -> impl IntoView {
    let people: RwSignal<Vec<UserProfile>> = RwSignal::new(vec![]);
    // Esc closes this dialog, not the details under it (the App handler leaves nested dialogs alone).
    scoped_window_listener(ev::keydown, move |e| {
        if e.key() == "Escape" {
            on_close.run(());
        }
    });
    let all_excluded = Signal::derive(move || {
        let mut v = exclude.get();
        v.extend(people.with(|p| p.iter().map(|x| x.id.clone()).collect::<Vec<_>>()));
        v
    });
    view! {
        <div class="modal-backdrop nested" on:click=move |ev| { ev.stop_propagation(); on_close.run(()); }>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{format!("{} #{title}", tr().add_people_to)}</h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| on_close.run(())><Icon icon=I::X/></button>
                </header>
                <PeoplePicker chat_id=chat_id exclude=all_excluded on_pick=Callback::new(move |p: UserProfile| people.update(|v| v.push(p)))/>
                <div class="chips">
                    {move || people.get().into_iter().map(|p| {
                        let id = p.id.clone();
                        view! {
                            <span class="chip"><Avatar user=profile_face(&p) size=20/>{p.display_name.clone()}
                                <button on:click=move |_| people.update(|v| v.retain(|x| x.id != id))><Icon icon=I::X size=12/></button>
                            </span>
                        }
                    }).collect_view()}
                </div>
                <div class="modal-actions">
                    <button class="btn primary" disabled=move || people.with(Vec::is_empty)
                        on:click=move |_| on_add.run(people.get_untracked().into_iter().map(|p| p.id).collect())>{tr().add_btn}</button>
                </div>
            </div>
        </div>
    }
}

/// The "DMs" view in the rail: all direct conversations with their last message, like DMs in Slack.
#[component]
fn DmList() -> impl IntoView {
    let c = ctx();
    let rows = Memo::new(move |_| {
        c.snap.with(|s| {
            let mut list: Vec<(Chat, String, Option<User>, Option<Message>)> = s
                .chats
                .iter()
                .filter(|ch| matches!(ch.kind, ChatKind::Direct { .. } | ChatKind::Notes))
                .map(|ch| {
                    let last = s
                        .messages
                        .iter()
                        .filter(|m| m.chat_id == ch.id && m.thread_parent.is_none())
                        .max_by_key(|m| m.ts)
                        .cloned();
                    (ch.clone(), display_title(s, ch), chat_face(s, ch), last)
                })
                .collect();
            list.sort_by_key(|(ch, _, _, last)| std::cmp::Reverse(last.as_ref().map_or(ch.last_ts, |m| m.ts)));
            list
        })
    });
    view! {
        <div class="dm-list">
            <div class="dm-list-title">{tr().sec_dms}</div>
            {move || rows.get().into_iter().map(|(ch, title, face, last)| {
                let status = face.as_ref().and_then(status_badge);
                let id = ch.id.clone();
                let ctx_id = ch.id.clone();
                let selected = c.current.with(|cur| cur == &id);
                let me = c.snap.with(|s| s.me.clone());
                let preview = last.as_ref().map(|m| {
                    if let Some(note) = &m.call {
                        let end = matches!(note, CallNote::Ended { .. }).then_some(m);
                        return format!("📞 {}", call_title(note, end, &me).0);
                    }
                    if let Some(text) = c.snap.with(|s| chat_note_text(s, m)) {
                        let who = c.snap.with(|s| s.user(&m.author).map(|u| u.display_name.clone())).unwrap_or_default();
                        return format!("{who} {text}");
                    }
                    let text = c.snap.with(|s| s.plain(&m.text));
                    let text = if text.is_empty() && !m.forwarded.is_empty() { format!("↪ {}", c.snap.with(|s| s.plain(&m.forwarded[0].text))) } else if text.is_empty() && !m.attachments.is_empty() { "📎".to_owned() } else { text };
                    if m.author == me && ch.kind != ChatKind::Notes { format!("{}: {text}", tr().you_label) } else { text }
                }).unwrap_or_default();
                let when = last.as_ref().map(|m| time::short_date(m.ts)).unwrap_or_default();
                let unread = ch.unread > 0 && !ch.muted;
                view! {
                    <button class=format!("dm-row{}{}", if selected { " selected" } else { "" }, if unread { " unread" } else { "" })
                        on:click=move |_| ops::open_chat(c, id.clone())
                        on:contextmenu=move |e: ev::MouseEvent| {
                            e.prevent_default();
                            c.ctx_menu.set(Some(CtxMenu { x: e.client_x() as f64, y: e.client_y() as f64, target: CtxTarget::Chat(ctx_id.clone()) }));
                        }>
                        {face.map(|u| view! { <Avatar user=u size=36 presence=true/> })}
                        <span class="dm-body">
                            <span class="dm-top">
                                <span class="dm-name">{title}{(ch.kind == ChatKind::Notes).then(|| view! { <span class="sb-you">{tr().you_short}</span> })}{status}</span>
                                <span class="dm-time">{when}</span>
                            </span>
                            <span class="dm-preview">{preview}</span>
                        </span>
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ActivityTab {
    All,
    Mentions,
    Threads,
    Reactions,
}

/// An event of the "Mentions" feed: a mention, a thread reply or reactions to your message.
#[derive(Clone, PartialEq)]
enum ActivityKind {
    Mention,
    Reply,
    Reactions(Vec<String>),
}

/// "Mentions" in the rail, like Activity in Slack: mentions of you, replies in your threads and reactions
/// to your messages, from loaded data and (in the app) from a search over recent chats.
#[component]
fn ActivityList() -> impl IntoView {
    let c = ctx();
    let tab = RwSignal::new(ActivityTab::All);
    let found: RwSignal<Vec<Message>> = RwSignal::new(vec![]);
    let loading = RwSignal::new(false);
    // In the app also search for mentions by name in recent chats (the server searches one chat at a time).
    if c.live {
        let name = c
            .snap
            .with_untracked(|s| s.user(&s.me).map(|u| u.display_name.clone()))
            .unwrap_or_default();
        if !name.is_empty() {
            loading.set(true);
            let chats: Vec<String> = c.snap.with_untracked(|s| {
                let mut list: Vec<&Chat> = s
                    .chats
                    .iter()
                    .filter(|ch| !matches!(ch.kind, ChatKind::Notes))
                    .collect();
                list.sort_by_key(|ch| std::cmp::Reverse(ch.last_ts));
                list.into_iter().take(20).map(|ch| ch.id.0.clone()).collect()
            });
            spawn_local(async move {
                if let Ok(f) = bridge::search_messages(&name, chats).await {
                    c.snap.update(|s| {
                        for u in f.users {
                            if s.user(&u.id).is_none() {
                                s.users.push(u);
                            }
                        }
                    });
                    found.try_set(f.messages);
                }
                loading.try_set(false);
            });
        }
    }
    let items = Memo::new(move |_| {
        let extra = found.get();
        c.snap.with(|s| {
            let me = s.me.clone();
            let my_name = s.user(&me).map(|u| u.display_name.clone()).unwrap_or_default();
            let names = s.mention_names();
            let my = Me {
                id: &me.0,
                name: &my_name,
            };
            let mut seen = HashSet::new();
            let mut out: Vec<(Message, ActivityKind)> = vec![];
            for m in s.messages.iter().chain(extra.iter()) {
                if !seen.insert((m.chat_id.clone(), m.ts)) {
                    continue;
                }
                if m.author != me && names.with(|who| mostik_core::markup::mentions_me(&m.text, who, my)) {
                    out.push((m.clone(), ActivityKind::Mention));
                } else if let Some(parent) = m.thread_parent
                    && m.author != me
                {
                    let mine = s.messages.iter().any(|p| {
                        p.chat_id == m.chat_id && p.author == me && (p.ts == parent || p.thread_parent == Some(parent))
                    });
                    if mine {
                        out.push((m.clone(), ActivityKind::Reply));
                    }
                }
                if m.author == me {
                    let others: Vec<String> = m
                        .reactions
                        .iter()
                        .filter(|r| r.count > u32::from(r.mine))
                        .map(|r| r.emoji.clone())
                        .collect();
                    if !others.is_empty() {
                        out.push((m.clone(), ActivityKind::Reactions(others)));
                    }
                }
            }
            out.sort_by_key(|(m, _)| std::cmp::Reverse(m.ts));
            out.truncate(100);
            out
        })
    });
    let tab_btn = move |t: ActivityTab, label: &'static str| {
        view! {
            <button class=move || if tab.get() == t { "act-tab active" } else { "act-tab" } on:click=move |_| tab.set(t)>{label}</button>
        }
    };
    view! {
        <div class="dm-list activity-list">
            <div class="dm-list-title">{tr().rail_activity}</div>
            <div class="act-tabs">
                {tab_btn(ActivityTab::All, tr().act_all)}
                {tab_btn(ActivityTab::Mentions, tr().act_mentions)}
                {tab_btn(ActivityTab::Threads, tr().act_threads)}
                {tab_btn(ActivityTab::Reactions, tr().act_reactions)}
            </div>
            {move || {
                let t = tab.get();
                let list: Vec<(Message, ActivityKind)> = items.get().into_iter().filter(|(_, k)| match t {
                    ActivityTab::All => true,
                    ActivityTab::Mentions => *k == ActivityKind::Mention,
                    ActivityTab::Threads => *k == ActivityKind::Reply,
                    ActivityTab::Reactions => matches!(k, ActivityKind::Reactions(_)),
                }).collect();
                if list.is_empty() {
                    return view! {
                        <div class="empty-state small">
                            {move || if loading.get() { tr().searching } else { tr().act_empty }}
                        </div>
                    }.into_any();
                }
                list.into_iter().map(|(m, kind)| {
                    let (author, chat) = c.snap.with(|s| (
                        s.user(&m.author).cloned(),
                        s.chat(&m.chat_id).map(|ch| display_title(s, ch)).unwrap_or_default(),
                    ));
                    let me_face = c.snap.with(|s| s.user(&s.me).cloned());
                    let face = if matches!(kind, ActivityKind::Reactions(_)) { me_face } else { author.clone() };
                    let what = match &kind {
                        ActivityKind::Mention => tr().act_mention_in(&chat),
                        ActivityKind::Reply => tr().act_reply_in(&chat),
                        ActivityKind::Reactions(e) => format!("{} {}", e.join(" "), tr().act_reacted_in(&chat)),
                    };
                    let who = author.map(|u| u.display_name).unwrap_or_default();
                    let text: String = c.snap.with(|s| s.plain(&m.text)).chars().take(140).collect();
                    let (id, ts, parent) = (m.chat_id.clone(), m.ts, m.thread_parent);
                    let selected = move || c.highlight.with(|h| h.as_ref().is_some_and(|(hc, hts)| *hc == id && *hts == ts));
                    let id2 = m.chat_id.clone();
                    view! {
                        <button class=move || if selected() { "dm-row act-row selected" } else { "dm-row act-row" }
                            on:click=move |_| ops::jump_to_message(c, id2.clone(), ts, parent)>
                            {face.map(|u| view! { <Avatar user=u size=36/> })}
                            <span class="dm-body">
                                <span class="dm-top"><span class="act-what">{what}</span><span class="dm-time">{time::short_date(ts)}</span></span>
                                <span class="dm-name">{who}</span>
                                <span class="dm-preview">{text}</span>
                            </span>
                        </button>
                    }
                }).collect_view().into_any()
            }}
        </div>
    }
}

#[derive(Clone, Copy, PartialEq)]
enum DraftsTab {
    Drafts,
    Sent,
}

/// Followed threads, like Slack's "Threads": the ones with new replies first, then by the last
/// reply. A click opens the thread in the side pane.
#[component]
fn ThreadsView() -> impl IntoView {
    let c = ctx();
    let list = Memo::new(move |_| {
        let mut list = c.threads.get();
        list.sort_by_key(|t| (t.unread == 0, std::cmp::Reverse(t.last_ts)));
        list
    });
    view! {
        <header class="ch-header">
            <div class="ch-title-row"><div class="ch-title"><span class="ch-kind"><Icon icon=I::Threads size=18/></span><span>{tr().threads}</span></div></div>
        </header>
        <div class="messages drafts-list">
            {move || {
                let list = list.get();
                if list.is_empty() {
                    return view! {
                        <div class="empty-state"><h3>{tr().no_threads}</h3><p>{tr().no_threads_hint}</p></div>
                    }.into_any();
                }
                let open = c.thread.get().map(|p| (c.current.get(), p));
                list.into_iter().map(|t| {
                    let (author, title, names, me_name, me_id) = c.snap.with(|s| (
                        s.user(&t.parent.author).cloned(),
                        s.chat(&t.parent.chat_id).map(|ch| display_title(s, ch)).unwrap_or_default(),
                        s.mention_names(),
                        s.user(&s.me).map(|u| u.display_name.clone()).unwrap_or_default(),
                        s.me.clone(),
                    ));
                    let name = author.as_ref().map_or_else(|| tr().user.to_owned(), |u| u.display_name.clone());
                    let text = names.with(|who| render_text(&t.parent.text, who, Me { id: &me_id.0, name: &me_name }));
                    // The parent loaded in its chat is the fresher copy: deletions lower its count.
                    let replies = c.snap.with(|s| {
                        s.messages
                            .iter()
                            .find(|m| m.chat_id == t.parent.chat_id && m.ts == t.parent.ts)
                            .and_then(|m| m.thread.as_ref())
                            .or(t.parent.thread.as_ref())
                            .map_or(0, |th| th.reply_count)
                    });
                    let selected = open.as_ref() == Some(&(t.parent.chat_id.clone(), t.parent.ts));
                    let unread = t.unread;
                    let has_unread = unread > 0;
                    let parent = t.parent.clone();
                    view! {
                        <div class="draft-row thread-row" class:unread=has_unread class:selected=selected
                            on:click=move |_| ops::open_followed_thread(c, parent.clone())>
                            <span class="draft-face">{match author {
                                Some(u) => view! { <Avatar user=u size=36/> }.into_any(),
                                None => view! { <span class="avatar unknown"></span> }.into_any(),
                            }}</span>
                            <span class="dm-body">
                                <span class="dm-top">
                                    <span class="dm-name">{name}</span>
                                    <span class="thread-where">{format!("# {title}")}</span>
                                    <span class="dm-time">{time::relative(t.last_ts)}</span>
                                </span>
                                <span class="thread-text">{text}</span>
                                <span class="thread-meta">
                                    {(replies > 0).then(|| tr().replies(replies))}
                                    {(unread > 0).then(|| view! { <span class="thread-new">{tr().new_replies(unread)}</span> })}
                                </span>
                            </span>
                        </div>
                    }
                }).collect_view().into_any()
            }}
        </div>
    }
}

/// "Drafts & sent", as in Slack.
#[component]
fn DraftsView() -> impl IntoView {
    let c = ctx();
    let tab = RwSignal::new(DraftsTab::Drafts);
    let go = move |id: ChatId| {
        c.view.set(ViewMode::Home);
        ops::open_chat(c, id);
    };
    let drafts = Memo::new(move |_| {
        let d = c.drafts.get();
        c.snap.with(|s| {
            d.into_iter()
                .filter(|(_, t)| !t.trim().is_empty())
                .filter_map(|(id, t)| {
                    s.chat(&id)
                        .map(|ch| (id.clone(), display_title(s, ch), chat_face(s, ch), Mark::of(ch), t))
                })
                .collect::<Vec<_>>()
        })
    });
    let sent = Memo::new(move |_| {
        c.snap.with(|s| {
            let mut list: Vec<(Message, String)> = s
                .messages
                .iter()
                .filter(|m| m.author == s.me && !m.is_note())
                .map(|m| {
                    (
                        m.clone(),
                        s.chat(&m.chat_id).map(|ch| display_title(s, ch)).unwrap_or_default(),
                    )
                })
                .collect();
            list.sort_by_key(|(m, _)| std::cmp::Reverse(m.ts));
            list.truncate(200);
            list
        })
    });
    view! {
        <header class="ch-header">
            <div class="ch-title-row"><div class="ch-title"><span class="ch-kind"><Icon icon=I::Send size=18/></span><span>{tr().drafts_and_sent}</span></div></div>
            <nav class="ch-tabs">
                <button class=move || if tab.get() == DraftsTab::Drafts { "tab active" } else { "tab" } on:click=move |_| tab.set(DraftsTab::Drafts)>
                    <Icon icon=I::Compose size=14/><span>{tr().drafts}</span>
                    {move || { let n = drafts.with(Vec::len); (n > 0).then(|| view! { <span class="tab-count">{n}</span> }) }}
                </button>
                <button class=move || if tab.get() == DraftsTab::Sent { "tab active" } else { "tab" } on:click=move |_| tab.set(DraftsTab::Sent)>
                    <Icon icon=I::Send size=14/><span>{tr().sent}</span>
                </button>
            </nav>
        </header>
        <div class="messages drafts-list">
            {move || match tab.get() {
                DraftsTab::Drafts => {
                    let list = drafts.get();
                    if list.is_empty() {
                        return view! { <div class="empty-state"><h3>{tr().no_drafts}</h3><p>{tr().no_drafts_hint}</p></div> }.into_any();
                    }
                    list.into_iter().map(|(id, title, face, kind, text)| {
                        let (open, del) = (id.clone(), id.clone());
                        view! {
                            <div class="draft-row" on:click=move |_| go(open.clone())>
                                <span class="draft-face">{match face {
                                    Some(u) => view! { <Avatar user=u size=36/> }.into_any(),
                                    None => view! { <span class="draft-kind">{kind_icon(&kind, 18)}</span> }.into_any(),
                                }}</span>
                                <span class="dm-body">
                                    <span class="dm-name">{title}</span>
                                    <span class="dm-preview">{c.snap.with(|s| s.plain(&text))}</span>
                                </span>
                                <button class="icon-btn" title=tr().delete_draft on:click=move |e| {
                                    e.stop_propagation();
                                    c.drafts.update(|d| { d.remove(&del); });
                                }><Icon icon=I::X size=16/></button>
                            </div>
                        }
                    }).collect_view().into_any()
                }
                DraftsTab::Sent => {
                    let list = sent.get();
                    if list.is_empty() {
                        return view! { <div class="empty-state"><h3>{tr().no_sent}</h3></div> }.into_any();
                    }
                    list.into_iter().map(|(m, title)| {
                        let id = m.chat_id.clone();
                        let text = c.snap.with(|s| s.plain(&m.text));
                        let text = if text.is_empty() && !m.forwarded.is_empty() { format!("↪ {}", c.snap.with(|s| s.plain(&m.forwarded[0].text))) } else if text.is_empty() && !m.attachments.is_empty() { "📎".to_owned() } else { text };
                        view! {
                            <div class="draft-row" on:click=move |_| go(id.clone())>
                                <span class="dm-body">
                                    <span class="dm-top"><span class="dm-name">{title}</span><span class="dm-time">{time::relative(m.ts)}</span></span>
                                    <span class="dm-preview">{text}</span>
                                </span>
                            </div>
                        }
                    }).collect_view().into_any()
                }
            }}
        </div>
    }
}

/// Avatar menu in the rail, as in Slack: name and status, "Update your status", away / active,
/// pausing notifications for a while, profile, preferences, sign out.
#[component]
fn MeMenu() -> impl IntoView {
    let c = ctx();
    let me = Memo::new(move |_| c.snap.with(|s| s.user(&s.me).cloned()));
    let pause_open = RwSignal::new(false);
    let close = move || c.me_menu.set(false);
    let set = move |availability: Availability, duration: Option<u64>| {
        close();
        let status = me.get_untracked().and_then(|u| u.status);
        ops::set_status(c, availability, status, duration);
    };
    // Paused here or on another device; an ended pause is cleared every second.
    let paused = Memo::new(move |_| c.snap.with(|s| s.paused));
    let pauses = move || {
        [
            (30 * 60, tr().pause_30m),
            (60 * 60, tr().pause_1h),
            (2 * 60 * 60, tr().pause_2h),
            (seconds_until_tomorrow_morning(), tr().pause_tomorrow),
        ]
    };
    view! {
        <div class="me-menu" on:mouseleave=move |_| close()>
            {move || me.get().map(|u| {
                let line = match (paused.get().and_then(|p| p.until), u.status.clone()) {
                    (Some(until), _) => format!("{} {}", tr().paused_until, time::hm(until)),
                    (_, Some(st)) if st.in_meeting => status_text(&st),
                    (_, Some(st)) => format!("{} {}", st.emoji, st.text).trim().to_owned(),
                    _ => availability_label(&u).to_owned(),
                };
                view! {
                    <div class="me-head">
                        <Avatar user=u.clone() size=36/>
                        <div>
                            <div class="me-name">{u.display_name.clone()}</div>
                            <div class="me-status"><span class=presence_class(&u)></span>{line}</div>
                        </div>
                    </div>
                }
            })}
            <button class="me-status-btn" on:click=move |_| { close(); c.status_dialog.set(true); }>
                <Icon icon=I::Smile size=16/><span>{tr().update_status}</span>
            </button>
            <div class="me-sep"></div>
            {move || {
                let away = me.get().is_some_and(|u| u.availability == Availability::Away);
                let dnd = paused.get().is_some() || me.get().is_some_and(|u| u.availability == Availability::DoNotDisturb);
                view! {
                    <button on:click=move |_| set(if away { Availability::Default } else { Availability::Away }, None)>
                        {if away { tr().set_active } else { tr().set_away }}
                    </button>
                    {if dnd {
                        view! { <button on:click=move |_| set(Availability::Default, None)>{tr().resume_notifications}</button> }.into_any()
                    } else {
                        view! {
                            <div class="me-sub" on:mouseenter=move |_| pause_open.set(true) on:mouseleave=move |_| pause_open.set(false)>
                                <button class="me-sub-btn" on:click=move |_| pause_open.set(true)>
                                    <span>{tr().pause_for}</span><Icon icon=I::ChevronRight size=14/>
                                </button>
                                <Show when=move || pause_open.get()>
                                    <div class="me-submenu">
                                        {pauses().into_iter().map(|(secs, label)| view! {
                                            <button on:click=move |_| set(Availability::DoNotDisturb, Some(secs))>{label}</button>
                                        }).collect_view()}
                                    </div>
                                </Show>
                            </div>
                        }.into_any()
                    }}
                }
            }}
            <div class="me-sep"></div>
            <button on:click=move |_| {
                close();
                let me = c.snap.with_untracked(|s| s.me.clone());
                ops::open_profile(c, me);
            }>{tr().profile}</button>
            <button on:click=move |_| { close(); c.prefs.set(true); }>{tr().preferences}</button>
            {c.live.then(|| view! {
                <div class="me-sep"></div>
                <button on:click=move |_| {
                    close();
                    ops::sign_out(c);
                }>{move || tr().sign_out_of(&org_name(c))}</button>
            })}
        </div>
    }
}

/// Until 9:00 the next day: "Until tomorrow" in the notification pause, as in Slack.
fn seconds_until_tomorrow_morning() -> u64 {
    seconds_until_midnight() + 9 * 3600
}

/// "Update your status", as in Slack: emoji, text, presets and when to clear it.
#[component]
fn StatusDialog() -> impl IntoView {
    let c = ctx();
    let current = c.snap.with_untracked(|s| s.user(&s.me).and_then(|u| u.status.clone()));
    let availability = c
        .snap
        .with_untracked(|s| s.user(&s.me).map(|u| u.availability).unwrap_or_default());
    let emoji = RwSignal::new(current.as_ref().map(|s| s.emoji.clone()).unwrap_or_default());
    let text = RwSignal::new(current.as_ref().map(|s| s.text.clone()).unwrap_or_default());
    let clear = RwSignal::new(0u64);
    let close = move || c.status_dialog.set(false);
    let save = move || {
        let (e, t) = (emoji.get_untracked(), text.get_untracked());
        let status = (!e.trim().is_empty() || !t.trim().is_empty()).then(|| CustomStatus {
            emoji: e.trim().to_owned(),
            text: t.trim().to_owned(),
            in_meeting: false,
        });
        let d = clear.get_untracked();
        ops::set_status(c, availability, status, (d > 0).then_some(d));
        close();
    };
    let clear_options: [(u64, &'static str); 5] = [
        (0, tr().clear_never),
        (30 * 60, tr().clear_30m),
        (60 * 60, tr().clear_1h),
        (4 * 60 * 60, tr().clear_4h),
        (seconds_until_midnight(), tr().clear_today),
    ];
    let emoji_at: RwSignal<Option<(f64, f64)>> = RwSignal::new(None);
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal status-modal" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{tr().update_status}</h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
                </header>
                <div class="status-input">
                    // The emoji is picked, as in Slack: a text field would need the OS emoji input.
                    <button class="emoji-input" title=tr().section_emoji
                        on:click=move |e| emoji_at.set(Some(crate::emoji::anchor_of(&e)))>
                        {move || match emoji.get() {
                            e if e.is_empty() => view! { <Icon icon=I::Smile size=20/> }.into_any(),
                            e => view! { <span>{e}</span> }.into_any(),
                        }}
                    </button>
                    <input class="text-input bare" maxlength="100" placeholder=tr().status_placeholder prop:value=move || text.get()
                        on:input=move |e| text.set(event_target_value(&e))/>
                    <Show when=move || !emoji.get().is_empty() || !text.get().is_empty()>
                        <button class="icon-btn" title=tr().clear_status on:click=move |_| { emoji.set(String::new()); text.set(String::new()); }>
                            <Icon icon=I::X size=14/>
                        </button>
                    </Show>
                </div>
                {move || emoji_at.get().map(|at| view! {
                    <crate::emoji::EmojiPicker source=crate::emoji::Source::Text anchor=(at.0 + 312.0, at.1)
                        on_close=Callback::new(move |_| emoji_at.set(None))
                        on_pick=Callback::new(move |glyph: String| {
                            emoji_at.set(None);
                            crate::emoji::remember(crate::emoji::Source::Text, &glyph);
                            emoji.set(glyph);
                        })/>
                })}
                <div class="field-label">{tr().for_your_org}</div>
                <div class="status-presets">
                    {tr().status_presets.into_iter().map(|(e, t)| view! {
                        <button class="status-preset" on:click=move |_| { emoji.set(e.to_owned()); text.set(t.to_owned()); }>
                            <span class="preset-emoji">{e}</span><span>{t}</span>
                        </button>
                    }).collect_view()}
                </div>
                <label class="field-label">{tr().clear_after}</label>
                <select class="pref-select" on:change=move |e| clear.set(event_target_value(&e).parse().unwrap_or(0))>
                    {clear_options.into_iter().map(|(secs, label)| view! { <option value=secs.to_string()>{label}</option> }).collect_view()}
                </select>
                <div class="modal-actions">
                    <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                    <button class="btn primary" on:click=move |_| save()>{tr().save}</button>
                </div>
            </div>
        </div>
    }
}

fn seconds_until_midnight() -> u64 {
    let now = js_sys::Date::new_0();
    let left = (23 - now.get_hours()) * 3600 + (59 - now.get_minutes()) * 60 + (60 - now.get_seconds());
    u64::from(left.max(60))
}

#[derive(Clone, Copy, PartialEq)]
pub enum PrefTab {
    Notifications,
    Appearance,
    Messages,
    Language,
    MarkRead,
    Home,
    Accessibility,
    Advanced,
    Updates,
    Emoji,
}

/// "Preferences" modelled on Slack's Preferences (only what applies to Telemost).
#[component]
fn PreferencesDialog() -> impl IntoView {
    let c = ctx();
    let tab = c.pref_tab;
    let tabs = [
        (PrefTab::Notifications, I::Bell, tr().p_notifications),
        (PrefTab::Appearance, I::Star, tr().p_appearance),
        (PrefTab::Messages, I::Chat, tr().p_messages_media),
        (PrefTab::Language, I::Globe, tr().p_language_region),
        (PrefTab::Home, I::Home, tr().p_home),
        (PrefTab::MarkRead, I::Check, tr().p_mark_as_read),
        (PrefTab::Accessibility, I::Users, tr().p_accessibility),
        (PrefTab::Advanced, I::Kebab, tr().p_advanced),
        (PrefTab::Emoji, I::Smile, tr().custom_emoji),
        (PrefTab::Updates, I::Download, tr().p_updates),
    ];
    view! {
        <div class="modal-backdrop" on:click=move |_| c.prefs.set(false)>
            <div class="modal prefs" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{tr().preferences}</h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| c.prefs.set(false)>
                        <Icon icon=I::X/>
                    </button>
                </header>
                <div class="prefs-body">
                    <nav class="prefs-nav">
                        {tabs.into_iter().map(|(id, icon, label)| view! {
                            <button class=move || if tab.get() == id { "active" } else { "" } on:click=move |_| tab.set(id)>
                                <Icon icon=icon size=16/><span>{label}</span>
                            </button>
                        }).collect_view()}
                    </nav>
                    <section class="prefs-section">
                        {move || match tab.get() {
                            PrefTab::Notifications => view! { <PrefNotifications/> }.into_any(),
                            PrefTab::Appearance => view! { <PrefAppearance/> }.into_any(),
                            PrefTab::Messages => view! { <PrefMessages/> }.into_any(),
                            PrefTab::Language => view! { <PrefLanguage/> }.into_any(),
                            PrefTab::Home => view! { <PrefHome/> }.into_any(),
                            PrefTab::MarkRead => view! { <PrefMarkRead/> }.into_any(),
                            PrefTab::Accessibility => view! { <PrefAccessibility/> }.into_any(),
                            PrefTab::Advanced => view! { <PrefAdvanced/> }.into_any(),
                            PrefTab::Updates => view! { <PrefUpdates/> }.into_any(),
                            PrefTab::Emoji => view! { <PrefEmoji/> }.into_any(),
                        }}
                    </section>
                </div>
            </div>
        </div>
    }
}

/// Radio button bound to a preferences field.
fn radio<T: PartialEq + Copy + Send + Sync + 'static>(
    c: Ctx,
    name: &'static str,
    label: &'static str,
    value: T,
    get: fn(&Settings) -> T,
    set: fn(&mut Settings, T),
) -> impl IntoView {
    view! {
        <label class="pref-radio">
            <input type="radio" name=name prop:checked=move || c.settings.with(|s| get(s) == value)
                on:change=move |_| c.settings.update(|s| set(s, value))/>
            <span>{label}</span>
        </label>
    }
}

fn checkbox(c: Ctx, label: &'static str, get: fn(&Settings) -> bool, set: fn(&mut Settings, bool)) -> impl IntoView {
    view! {
        <label class="pref-check">
            <input type="checkbox" prop:checked=move || c.settings.with(get)
                on:change=move |e| { let v = event_target_checked(&e); c.settings.update(|s| set(s, v)) }/>
            <span>{label}</span>
        </label>
    }
}

#[component]
fn PrefNotifications() -> impl IntoView {
    let c = ctx();
    let example = move |_| {
        if !c.live {
            return c.error.set(Some(tr().only_in_app_notifications.into()));
        }
        ops::play_sound(c, c.settings.with_untracked(|s| s.sound.clone()));
        spawn_local(async move {
            if let Err(e) = bridge::test_notification().await {
                c.error.set(Some(e));
            }
        });
    };
    let n = |s: &Settings| s.notify;
    let set_n = |s: &mut Settings, v| s.notify = v;
    view! {
        <h3>{tr().p_notify_about}</h3>
        {radio(c, "notify", tr().p_notify_all, NotifyMode::All, n, set_n)}
        {radio(c, "notify", tr().p_notify_direct, NotifyMode::DirectAndMentions, n, set_n)}
        {radio(c, "notify", tr().p_notify_nothing, NotifyMode::Nothing, n, set_n)}
        <div class="pref-gap"></div>
        {checkbox(c, tr().p_notify_preview, |s| s.notify_preview, |s, v| s.notify_preview = v)}
        <h3 class="pref-h">{tr().p_sound}</h3>
        <select class="pref-select" on:change=move |e| {
            let v = event_target_value(&e);
            ops::play_sound(c, v.clone());
            c.settings.update(|s| s.sound = v);
        }>
            {mostik_core::NOTIFICATION_SOUNDS.into_iter().map(|id| view! {
                <option value=id selected=move || c.settings.with(|s| s.sound == id)>{tr().sound_name(id)}</option>
            }).collect_view()}
        </select>
        <p class="pref-hint">{tr().p_notify_explain}</p>
        <button class="link" on:click=example>{tr().p_show_example}</button>
    }
}

#[component]
fn PrefAppearance() -> impl IntoView {
    let c = ctx();
    let m = |s: &Settings| s.color_mode;
    let set_m = |s: &mut Settings, v| s.color_mode = v;
    view! {
        <h3>{tr().p_color_mode}</h3>
        <div class="pref-row">
            {radio(c, "mode", tr().p_mode_light, ColorMode::Light, m, set_m)}
            {radio(c, "mode", tr().p_mode_dark, ColorMode::Dark, m, set_m)}
            {radio(c, "mode", tr().p_mode_system, ColorMode::System, m, set_m)}
        </div>
        <h3 class="pref-h">{tr().p_themes}</h3>
        <div class="theme-group">{tr().themes_own}</div>
        {theme_buttons(c, 0..OWN_THEMES)}
        <div class="theme-group">{tr().themes_classic}</div>
        {theme_buttons(c, OWN_THEMES..THEMES.len())}
    }
}

/// Buttons of the themes `range` of [`THEMES`]: a swatch and the name.
fn theme_buttons(c: Ctx, range: std::ops::Range<usize>) -> impl IntoView {
    view! {
        <div class="themes">
            {range.map(|i| {
                let id = THEMES[i];
                view! {
                    <button class=move || if c.settings.with(|s| s.theme == id) { "theme selected" } else { "theme" }
                        on:click=move |_| c.settings.update(|s| s.theme = id.to_owned())>
                        <span class="swatch" data-theme=id></span>
                        <span>{tr().theme_names[i]}</span>
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn PrefMessages() -> impl IntoView {
    let c = ctx();
    view! {
        <h3>{tr().p_messages_media}</h3>
        {checkbox(c, tr().p_inline_media, |s| s.inline_media, |s, v| s.inline_media = v)}
        <p class="pref-hint">{tr().p_inline_media_hint}</p>
        <h3 class="pref-h">{tr().p_clock}</h3>
        {checkbox(c, tr().p_clock24, |s| s.clock24, |s, v| s.clock24 = v)}
    }
}

/// "Accessibility": UI zoom, like the Accessibility → Zoom section in Slack.
#[component]
fn PrefAccessibility() -> impl IntoView {
    let c = ctx();
    view! {
        <h3>{tr().p_zoom}</h3>
        <select class="pref-select" on:change=move |e| {
            if let Ok(z) = event_target_value(&e).parse::<u32>() {
                c.settings.update(|s| s.zoom = z);
            }
        }>
            {crate::settings::ZOOM_LEVELS.into_iter().map(|z| view! {
                <option value=z.to_string() selected=move || c.settings.with(|s| s.zoom == z)>{format!("{z}%")}</option>
            }).collect_view()}
        </select>
        <p class="pref-hint">{tr().p_zoom_hint}</p>
    }
}

#[component]
fn PrefLanguage() -> impl IntoView {
    let c = ctx();
    view! {
        <h3>{tr().p_language}</h3>
        <select class="pref-select" on:change=move |e| {
            let lang = if event_target_value(&e) == "en" { Lang::En } else { Lang::Ru };
            c.settings.update(|s| s.lang = lang);
        }>
            <option value="ru" selected=move || c.settings.with(|s| s.lang == Lang::Ru)>"Русский"</option>
            <option value="en" selected=move || c.settings.with(|s| s.lang == Lang::En)>"English"</option>
        </select>
        <p class="pref-hint">{tr().p_language_hint}</p>
    }
}

/// "Home", like Preferences → Home in Slack: filter, sorting and muted conversations.
#[component]
fn PrefHome() -> impl IntoView {
    let c = ctx();
    let gf = |s: &Settings| s.sidebar_filter;
    let sf = |s: &mut Settings, v| s.sidebar_filter = v;
    let gs = |s: &Settings| s.sidebar_sort;
    let ss = |s: &mut Settings, v| s.sidebar_sort = v;
    view! {
        <h3>{tr().filter_by}</h3>
        {radio(c, "sb-filter", tr().f_active, SidebarFilter::Active, gf, sf)}
        <p class="pref-hint">{tr().p_active_hint}</p>
        {radio(c, "sb-filter", tr().f_unreads, SidebarFilter::Unreads, gf, sf)}
        {radio(c, "sb-filter", tr().f_mentions, SidebarFilter::Mentions, gf, sf)}
        {radio(c, "sb-filter", tr().f_all, SidebarFilter::All, gf, sf)}
        <h3 class="pref-h">{tr().sort_by}</h3>
        {radio(c, "sb-sort", tr().s_az, SidebarSort::Alphabetical, gs, ss)}
        {radio(c, "sb-sort", tr().s_recency, SidebarSort::Recency, gs, ss)}
        {radio(c, "sb-sort", tr().s_custom, SidebarSort::Custom, gs, ss)}
        <h3 class="pref-h">{tr().p_muted}</h3>
        {checkbox(c, tr().hide_muted, |s| s.hide_muted, |s, v| s.hide_muted = v)}
        <h3 class="pref-h">{tr().p_sections}</h3>
        {checkbox(c, tr().p_group_calls_apart, |s| s.group_calls_apart, |s, v| s.group_calls_apart = v)}
        <p class="pref-hint">{tr().p_group_calls_hint}</p>
    }
}

#[component]
fn PrefMarkRead() -> impl IntoView {
    let c = ctx();
    let g = |s: &Settings| s.mark_read_on_open;
    let set = |s: &mut Settings, v| s.mark_read_on_open = v;
    view! {
        <h3>{tr().p_when_view}</h3>
        {radio(c, "read", tr().p_read_on_open, true, g, set)}
        {radio(c, "read", tr().p_leave_unread, false, g, set)}
        <h3 class="pref-h">{tr().p_read_receipts}</h3>
        {checkbox(c, tr().p_show_read_receipts, |s| s.show_read_receipts, |s, v| s.show_read_receipts = v)}
    }
}

/// "Updates": the background check switch, the installed version, a manual check and the install.
#[component]
fn PrefUpdates() -> impl IntoView {
    let c = ctx();
    // Result of the last manual check: `Some(Ok)` is "up to date", `Some(Err)` the error text.
    let checked: RwSignal<Option<Result<(), String>>> = RwSignal::new(None);
    let checking = RwSignal::new(false);
    let check = move |_| {
        checking.set(true);
        checked.set(None);
        spawn_local(async move {
            let result = bridge::check_update().await;
            checking.try_set(false);
            checked.try_set(Some(result.map(|status| c.update.set(status))));
        });
    };
    let status = move || {
        let status = if c.live {
            c.update.get()
        } else {
            bridge::UpdateStatus::Unsupported
        };
        match status {
            bridge::UpdateStatus::Unsupported => view! { <p class="pref-hint flush">{tr().update_unsupported}</p> }.into_any(),
            bridge::UpdateStatus::Available { version } => view! {
                <p>{tr().update_available(&version)}</p>
                <div class="pref-actions">
                    <button class="btn primary small" on:click=move |_| ops::install_update(c)>{tr().update_button}</button>
                </div>
            }
            .into_any(),
            bridge::UpdateStatus::Installing { percent, .. } => view! {
                <div class="pref-actions">
                    <button class="btn primary small" disabled=true>{tr().updating(percent)}</button>
                </div>
            }
            .into_any(),
            bridge::UpdateStatus::None => view! {
                {move || match checked.get() {
                    Some(Ok(())) => Some(view! { <p>{tr().up_to_date}</p> }.into_any()),
                    Some(Err(e)) => Some(view! { <p class="pref-error">{e}</p> }.into_any()),
                    None => None,
                }}
                <div class="pref-actions">
                    <button class="btn small" disabled=move || checking.get() on:click=check>
                        {move || if checking.get() { tr().checking_updates } else { tr().check_updates }}
                    </button>
                </div>
            }
            .into_any(),
        }
    };
    view! {
        <h3>{tr().p_updates}</h3>
        {checkbox(c, tr().p_auto_update, |s| s.auto_update, |s, v| s.auto_update = v)}
        <p class="pref-hint">{tr().p_auto_update_hint}</p>
        <h3 class="pref-h">{tr().version(env!("CARGO_PKG_VERSION"))}</h3>
        {status}
    }
}

/// Custom emoji of the organization, as in Slack's "Customize workspace": "Add emoji" opens the
/// same dialog as the emoji picker; your own can be renamed or deleted.
#[component]
fn PrefEmoji() -> impl IntoView {
    let c = ctx();
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    // The emoji being renamed (its message time) with the new name, and the one to confirm deleting.
    let renaming: RwSignal<Option<(Timestamp, String)>> = RwSignal::new(None);
    let deleting: RwSignal<Option<Timestamp>> = RwSignal::new(None);
    // Runs once Telemost answers, maybe after the preferences closed.
    let done = move |r: Result<Vec<mostik_core::custom_emoji::EmojiSet>, String>| match r {
        Ok(sets) => {
            c.custom.set(sets);
            renaming.try_set(None);
            deleting.try_set(None);
        }
        Err(e) => {
            error.try_set(Some(e));
        }
    };
    let row = move |e: mostik_core::custom_emoji::CustomEmoji, chat: ChatId, taken: bool| {
        let (me, author) = c.snap.with_untracked(|s| {
            let author = s
                .user(&e.owner)
                .filter(|u| u.has_name())
                .map_or_else(|| tr().unknown_user.to_owned(), |u| u.display_name.clone());
            (s.me.clone(), author)
        });
        let mine = e.owner == me;
        let ts = e.ts;
        let code = mostik_core::custom_emoji::caption(&e.name);
        let old = e.name.clone();
        let (chat_r, chat_d) = (chat.clone(), chat);
        let save = move || {
            let Some((_, n)) = renaming.get_untracked() else { return };
            let chat = chat_r.clone();
            spawn_local(async move { done(bridge::rename_custom_emoji(&chat.0, ts, n.trim()).await) });
        };
        let save_click = save.clone();
        let remove = move |_| {
            let chat = chat_d.clone();
            spawn_local(async move { done(bridge::delete_custom_emoji(&chat.0, ts).await) });
        };
        let actions = mine.then(|| view! {
            <span class="emoji-actions">
                {move || match (renaming.get().is_some_and(|(t, _)| t == ts), deleting.get() == Some(ts)) {
                    (true, _) => {
                        let save = save_click.clone();
                        view! {
                            <button class="btn small" on:click=move |_| renaming.set(None)>{tr().cancel}</button>
                            <button class="btn primary small" on:click=move |_| save()>{tr().save}</button>
                        }.into_any()
                    }
                    (_, true) => {
                        let remove = remove.clone();
                        view! {
                            <button class="btn small" on:click=move |_| deleting.set(None)>{tr().cancel}</button>
                            <button class="btn danger small" on:click=remove>{tr().delete}</button>
                        }.into_any()
                    }
                    _ => {
                        let old = old.clone();
                        view! {
                            <button class="btn small" on:click=move |_| { deleting.set(None); renaming.set(Some((ts, old.clone()))) }>{tr().emoji_rename}</button>
                            <button class="btn small" on:click=move |_| { renaming.set(None); deleting.set(Some(ts)) }>{tr().delete}</button>
                        }.into_any()
                    }
                }}
            </span>
        });
        let label = move || match renaming.get().filter(|(t, _)| *t == ts) {
            Some((_, n)) => {
                let save = save.clone();
                view! {
                    <input class="text-input emoji-name-input" prop:value=n
                        on:input=move |ev| renaming.set(Some((ts, event_target_value(&ev))))
                        on:keydown=move |ev: ev::KeyboardEvent| if ev.key() == "Enter" { ev.prevent_default(); save() }/>
                }
                .into_any()
            }
            None => view! { <span class="emoji-code">{code.clone()}</span> }.into_any(),
        };
        view! {
            <div class="emoji-row">
                {crate::custom::image(&e, "emoji-preview")}
                <span class="emoji-label">
                    {label}
                    <span class="emoji-author">{format!("{} {author}", tr().emoji_author)}</span>
                    {taken.then(|| view! { <span class="emoji-taken">{tr().emoji_taken}</span> })}
                </span>
                {actions}
            </div>
        }
    };
    let list = move || {
        let me = c.snap.with(|s| s.me.clone());
        match crate::custom::current(c) {
            Some(set) if !set.emoji.is_empty() || set.taken.iter().any(|e| e.owner == me) => {
                let chat = set.chat_id.clone();
                let taken: Vec<_> = set.taken.into_iter().filter(|e| e.owner == me).collect();
                // Your own first, newest on top; then the others', newest on top.
                let mut emoji = set.emoji;
                emoji.sort_by_key(|e| (e.owner != me, std::cmp::Reverse(e.ts)));
                view! {
                    <div class="emoji-list">
                        {taken.into_iter().map(|e| row(e, chat.clone(), true)).collect_view()}
                        {emoji.into_iter().map(|e| row(e, chat.clone(), false)).collect_view()}
                    </div>
                }
                .into_any()
            }
            _ => view! { <p class="pref-hint flush">{tr().no_custom_emoji}</p> }.into_any(),
        }
    };
    view! {
        <h3>{tr().custom_emoji}</h3>
        <p class="pref-hint flush">{tr().custom_emoji_hint}</p>
        {(!c.live).then(|| view! { <p class="pref-error">{tr().emoji_offline}</p> })}
        <div class="pref-actions">
            <button class="btn primary small" disabled=!c.live on:click=move |_| c.adding_emoji.set(true)>{tr().add_emoji}</button>
        </div>
        {move || error.get().map(|e| view! { <p class="pref-error">{e}</p> })}
        {list}
    }
}

/// "Add custom emoji", as in Slack: opened by "Add emoji" at the bottom of the emoji picker.
/// The emoji goes to the organization's service chat and shows up in the picker's custom section.
#[component]
fn AddEmojiDialog() -> impl IntoView {
    let c = ctx();
    let file: RwSignal<Option<bridge::PendingFile>> = RwSignal::new(None);
    let name = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let close = move || c.adding_emoji.set(false);
    let pick = move |_| {
        spawn_local(async move {
            // The dialog may be closed while the file dialog is open: its signals are gone then.
            match bridge::pick_emoji_image().await {
                Ok(Some(f)) => {
                    if name.try_with_untracked(|n| n.trim().is_empty()) == Some(true) {
                        name.try_set(mostik_core::custom_emoji::name_from_file(&f.name));
                    }
                    file.try_set(Some(f));
                    error.try_set(None);
                }
                Ok(None) => {}
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };
    let save = move || {
        if !c.live {
            return error.set(Some(tr().emoji_offline.into()));
        }
        let Some(f) = file.get_untracked() else { return };
        let n = name.get_untracked().trim().trim_matches(':').to_owned();
        let org = current_org(c).map(|o| o.id);
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match bridge::add_custom_emoji(&f.path, &n, org).await {
                Ok(sets) => {
                    c.custom.set(sets);
                    close();
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
            busy.try_set(false);
        });
    };
    let ready =
        move || !busy.get() && file.with(Option::is_some) && name.with(|n| !n.trim().trim_matches(':').is_empty());
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal add-emoji" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{tr().add_emoji_title}</h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
                </header>
                <p class="muted">{tr().custom_emoji_hint}</p>
                <label class="field-label">{tr().emoji_upload_step}</label>
                <div class="add-emoji-upload">
                    <div class="add-emoji-preview">
                        {move || file.get().map(|f| view! { <img src=bridge::local_file_url(&f.path) alt=""/> })}
                    </div>
                    <div>
                        <p class="muted">{tr().emoji_image_hint}</p>
                        <button class="btn small" disabled=move || busy.get() on:click=pick>
                            {move || if file.with(Option::is_some) { tr().emoji_change_image } else { tr().emoji_choose_image }}
                        </button>
                    </div>
                </div>
                <label class="field-label">{tr().emoji_name_step}</label>
                <div class="name-input"><span>":"</span>
                    <input class="text-input bare" placeholder=tr().emoji_name_ph maxlength="40" prop:value=move || name.get()
                        on:input=move |e| name.set(event_target_value(&e))
                        on:keydown=move |e: ev::KeyboardEvent| if e.key() == "Enter" && ready() { e.prevent_default(); save() }/>
                    <span>":"</span>
                </div>
                <p class="muted">{tr().emoji_name_hint}</p>
                {move || error.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                <div class="modal-actions">
                    <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                    <button class="btn primary" disabled=move || !ready() on:click=move |_| save()>
                        {move || if busy.get() { tr().emoji_adding } else { tr().save }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn PrefAdvanced() -> impl IntoView {
    let c = ctx();
    let g = |s: &Settings| s.enter_sends;
    let set = |s: &mut Settings, v| s.enter_sends = v;
    view! {
        <h3>{tr().p_enter_to}</h3>
        {radio(c, "enter", tr().p_enter_send, true, g, set)}
        {radio(c, "enter", tr().p_enter_newline, false, g, set)}
    }
}

/// "Profile" panel, as in Slack: a large photo, name, title, buttons, contacts.
#[component]
fn ProfilePane() -> impl IntoView {
    let c = ctx();
    let data = Memo::new(move |_| {
        let id = c.profile.get()?;
        let user = c.snap.with(|s| s.user(&id).cloned());
        let me = c.snap.with(|s| s.me == id);
        let profile = c.profiles.with(|p| p.get(&id).cloned());
        Some((id, user, profile, me))
    });
    view! {
        <aside class="thread-pane profile-pane">
            <header class="tp-header">
                <div class="tp-title">{tr().profile}</div>
                <button class="icon-btn" title=tr().close on:click=move |_| c.profile.set(None)><Icon icon=I::X/></button>
            </header>
            <div class="tp-scroll">
                {move || data.get().map(|(id, user, profile, me)| {
                    let name = profile
                        .as_ref()
                        .map(|p| p.display_name.clone())
                        .filter(|n| !n.is_empty())
                        .or_else(|| user.as_ref().map(|u| u.display_name.clone()))
                        .unwrap_or_else(|| tr().user.to_owned());
                    let fallback = User {
                        id: id.clone(),
                        display_name: name.clone(),
                        title: None,
                        presence: Presence::Offline,
                        status: None,
                        availability: Default::default(),
                        is_robot: false,
                        avatar: None,
                    };
                    let base = user.clone().unwrap_or(fallback);
                    let face = User { avatar: base.avatar.clone().or_else(|| profile.as_ref().and_then(|p| p.avatar.clone())), ..base };
                    let title = profile.as_ref().and_then(|p| p.position.clone()).or_else(|| user.as_ref().and_then(|u| u.title.clone()));
                    let department = profile.as_ref().and_then(|p| p.department.clone());
                    let organization = profile.as_ref().and_then(|p| p.organization.clone());
                    let email = profile.as_ref().and_then(|p| p.email.clone());
                    let nickname = profile.as_ref().and_then(|p| p.nickname.clone());
                    let msg_id = id.clone();
                    let call_id = id.clone();
                    view! {
                        <div class="profile">
                            <div class="profile-photo"><Avatar user=face size=300/></div>
                            <div class="profile-name-row">
                                <h2 class="profile-name">{name}</h2>
                                {(me && c.live).then(|| view! {
                                    <button class="link" on:click=move |_| spawn_local(async move {
                                        if let Err(e) = bridge::open_yandex_id().await { c.error.set(Some(e)); }
                                    })>{tr().edit_profile}</button>
                                })}
                            </div>
                            {title.map(|t| view! { <div class="profile-title">{t}</div> })}
                            {user.as_ref().and_then(|u| u.status.clone()).map(|st| view! {
                                <div class="profile-line"><span class="preset-emoji">{status_icon(&st, 18)}</span>{status_text(&st)}</div>
                            })}
                            {user.clone().map(|u| view! {
                                <div class="profile-line"><span class=presence_class(&u)></span>{availability_label(&u)}</div>
                            })}
                            {me.then(|| view! {
                                <div class="profile-line"><Icon icon=I::Clock size=16/>
                                    {move || format!("{} {}", time::hm((c.now.get() * 1000.0) as Timestamp), tr().local_time)}
                                </div>
                            })}
                            {(!me).then(|| view! {
                                <div class="profile-actions">
                                    <button class="btn" on:click=move |_| ops::message_user(c, msg_id.clone())>
                                        <Icon icon=I::Chat size=16/><span>{tr().message_button}</span>
                                    </button>
                                    <button class="btn" on:click={let id = call_id.clone(); move |_| ops::call_user(c, id.clone())}>
                                        <Icon icon=I::Video size=16/><span>{tr().call}</span>
                                    </button>
                                </div>
                            })}
                        </div>
                        {(email.is_some() || nickname.is_some()).then(|| view! {
                        <div class="profile-block">
                            <h3>{tr().contact_info}</h3>
                            {email.map(|e| view! {
                                <div class="profile-field">
                                    <span class="pf-icon"><Icon icon=I::Mail size=18/></span>
                                    <div><div class="pf-label">{tr().email_address}</div><div class="pf-value link-text">{e}</div></div>
                                </div>
                            })}
                            {nickname.map(|n| view! {
                                <div class="profile-field">
                                    <span class="pf-icon"><Icon icon=I::At size=18/></span>
                                    <div><div class="pf-label">{tr().nickname}</div><div class="pf-value">{n}</div></div>
                                </div>
                            })}
                        </div>
                        })}
                        {(department.is_some() || organization.is_some()).then(|| view! {
                            <div class="profile-block">
                                {organization.map(|o| view! {
                                    <div class="profile-field">
                                        <span class="pf-icon"><Icon icon=I::Users size=18/></span>
                                        <div><div class="pf-label">{tr().organization}</div><div class="pf-value">{o}</div></div>
                                    </div>
                                })}
                                {department.map(|d| view! {
                                    <div class="profile-field">
                                        <span class="pf-icon"><Icon icon=I::Home size=18/></span>
                                        <div><div class="pf-label">{tr().department}</div><div class="pf-value">{d}</div></div>
                                    </div>
                                })}
                            </div>
                        })}
                    }
                })}
            </div>
        </aside>
    }
}

#[component]
fn DeleteDialog() -> impl IntoView {
    let c = ctx();
    let close = move || c.confirm_delete.set(None);
    let confirm = move |_| {
        if let Some(m) = c.confirm_delete.get_untracked() {
            ops::delete(c, m);
        }
        close();
    };
    let preview = move || {
        c.confirm_delete
            .get()
            .map(|m| c.snap.with(|s| s.plain(&m.text)).chars().take(200).collect::<String>())
    };
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <header><h2>{tr().delete_message}</h2></header>
                <p>{tr().delete_message_warning}</p>
                <blockquote class="quote">{preview}</blockquote>
                <div class="modal-actions">
                    <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                    <button class="btn danger" on:click=confirm>{tr().delete}</button>
                </div>
            </div>
        </div>
    }
}

/// Leave-channel confirmation opened from the sidebar menu.
#[component]
fn LeaveDialog(id: ChatId) -> impl IntoView {
    let c = ctx();
    let close = move || c.confirm_leave.set(None);
    let title = c
        .snap
        .with_untracked(|s| s.chat(&id).map(|ch| display_title(s, ch)))
        .unwrap_or_default();
    let confirm = move |_| {
        close();
        ops::leave_chat(c, id.clone());
    };
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <header><h2>{tr().leave_title(&title)}</h2></header>
                <p>{tr().leave_confirm}</p>
                <div class="modal-actions">
                    <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                    <button class="btn danger" on:click=confirm>{tr().leave_channel}</button>
                </div>
            </div>
        </div>
    }
}

/// "New section" / "Rename section": the name of an own sidebar section.
#[component]
fn FolderDialog(edit: FolderEdit) -> impl IntoView {
    let c = ctx();
    let close = move || c.folder_edit.set(None);
    let renaming = matches!(edit, FolderEdit::Rename(_));
    let initial = match &edit {
        FolderEdit::Rename(id) => c
            .folders
            .with_untracked(|f| f.iter().find(|f| &f.id == id).map(|f| f.name.clone()))
            .unwrap_or_default(),
        FolderEdit::New(_) => String::new(),
    };
    let initial_emoji = match &edit {
        FolderEdit::Rename(id) => c
            .folders
            .with_untracked(|f| f.iter().find(|f| &f.id == id).and_then(|f| f.emoji.clone())),
        FolderEdit::New(_) => None,
    };
    let name = RwSignal::new(initial);
    let emoji = RwSignal::new(initial_emoji);
    let emoji_at: RwSignal<Option<(f64, f64)>> = RwSignal::new(None);
    let field = NodeRef::<html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = field.get() {
            // Once the value is in: the old name is selected, ready to be typed over.
            set_timeout(
                move || {
                    let _ = el.focus();
                    el.select();
                },
                Duration::ZERO,
            );
        }
    });
    let submit = move || {
        let text = name.get_untracked().trim().to_owned();
        if text.is_empty() {
            return;
        }
        close();
        let emoji = emoji.get_untracked();
        match edit.clone() {
            FolderEdit::Rename(id) => ops::change_folders(c, |f| {
                if let Some(f) = f.iter_mut().find(|f| f.id == id) {
                    f.name = text;
                    f.emoji = emoji;
                }
            }),
            FolderEdit::New(chat) => ops::change_folders(c, |f| {
                let id = folders::new_id(f, ops::random_u64());
                f.push(folders::Folder {
                    id: id.clone(),
                    name: text,
                    emoji,
                    chats: vec![],
                });
                if let Some(chat) = chat {
                    folders::move_chat(f, &chat, Some(&id), None);
                }
            }),
        }
    };
    let on_enter = submit.clone();
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <header><h2>{if renaming { tr().rename_section } else { tr().new_section_title }}</h2></header>
                <label class="field-label">{tr().section_name}</label>
                // The smiley picks the section's emoji, as in Slack.
                <div class="name-input">
                    <button class="section-emoji-btn" title=tr().section_emoji
                        on:click=move |e| emoji_at.set(Some(crate::emoji::anchor_of(&e)))>
                        {move || match emoji.get() {
                            Some(e) => view! { <span class="section-emoji">{e}</span> }.into_any(),
                            None => view! { <Icon icon=I::Smile size=18/> }.into_any(),
                        }}
                    </button>
                    <input class="text-input bare" node_ref=field maxlength="80" placeholder=tr().section_name_ph
                        prop:value=move || name.get()
                        on:input=move |e| name.set(event_target_value(&e))
                        on:keydown=move |e: ev::KeyboardEvent| {
                            if e.key() == "Enter" {
                                e.prevent_default();
                                on_enter();
                            }
                        }/>
                    {move || emoji.get().is_some().then(|| view! {
                        <button class="icon-btn section-emoji-clear" title=tr().remove_emoji on:click=move |_| emoji.set(None)>
                            <Icon icon=I::X size=14/>
                        </button>
                    })}
                </div>
                {move || emoji_at.get().map(|at| view! {
                    <crate::emoji::EmojiPicker source=crate::emoji::Source::Text anchor=(at.0 + 324.0, at.1)
                        on_close=Callback::new(move |_| emoji_at.set(None))
                        on_pick=Callback::new(move |glyph: String| {
                            emoji_at.set(None);
                            crate::emoji::remember(crate::emoji::Source::Text, &glyph);
                            emoji.set(Some(glyph));
                        })/>
                })}
                <div class="modal-actions">
                    <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                    <button class="btn primary" disabled=move || name.with(|n| n.trim().is_empty()) on:click=move |_| submit()>
                        {if renaming { tr().save } else { tr().create }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn AboutDialog() -> impl IntoView {
    let c = ctx();
    view! {
        <div class="modal-backdrop" on:click=move |_| c.about.set(false)>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <header>
                    <h2>{tr().about_title}</h2>
                    <button class="icon-btn" title=tr().close on:click=move |_| c.about.set(false)>
                        <Icon icon=I::X/>
                    </button>
                </header>
                <p><b>{tr().about_bold}</b></p>
                <p>{tr().about_trademarks}</p>
                <p>{tr().about_protocol}</p>
                <p class="muted">{tr().version(env!("CARGO_PKG_VERSION"))}</p>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_tip_shows_text() {
        let st = |emoji: &str, text: &str, in_meeting| CustomStatus {
            emoji: emoji.into(),
            text: text.into(),
            in_meeting,
        };
        assert_eq!(status_tip(&st("🏖", "Vacation", false)).as_deref(), Some("🏖 Vacation"));
        assert_eq!(status_tip(&st("", "Busy", false)).as_deref(), Some("Busy"));
        assert_eq!(status_tip(&st("👾", "", false)), None);
        assert_eq!(
            status_tip(&st("📞", "На встрече", true)).as_deref(),
            Some(tr().in_meeting)
        );
    }

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
        let mut s = mostik_core::mock::demo();
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
