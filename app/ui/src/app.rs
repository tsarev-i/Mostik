//! The main window: `App` sets up the shared state and its effects and shows the login screen, the
//! splash or the workspace with the dialogs over it. The parts of the window are in the submodules.

mod avatar;
mod composer;
mod dialogs;
mod feed;
mod field;
mod menus;
mod message;
mod org;
mod preferences;
mod profile;
mod rail;
mod sidebar;
mod state;
mod text;
mod topbar;
mod views;

use std::collections::HashSet;
use std::time::Duration;

use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::i18n::tr;
use crate::settings::Settings;
use crate::{bridge, folders, hidden, ops, order, storage};

// Paths the rest of the UI uses as `crate::app::…`.
pub(crate) use avatar::{Avatar, display_title};
pub(crate) use field::{insert_into, place_caret, selection, utf16_len, utf16_to_byte, wrap_selection};
pub(crate) use org::current_org;
pub use org::org_key;
pub use state::{Ctx, Phase, ViewMode, enter_sends};

use avatar::HoverTip;
use composer::Composer;
use dialogs::{
    AboutDialog, BrowseChannelsDialog, ChannelDetailsDialog, CreateChannelDialog, DeleteDialog, FolderDialog,
    ForwardDialog, LeaveDialog, NewMessageDialog,
};
use feed::{ChannelHeader, MessageList, PinsView};
use menus::ContextMenu;
use org::ORG_KEY;
use preferences::{AddEmojiDialog, PrefTab, PreferencesDialog};
use profile::{ProfilePane, StatusDialog};
use rail::Rail;
use sidebar::{Sidebar, chat_drag_to, chat_drop};
use state::{
    Drag, MAIN_MIN, RAIL_W, SIDEBAR_RANGE, THREAD_MIN, apply_settings, ctx, empty_snapshot, load_drafts, save_drafts,
    set_locale, store_width, stored_width,
};
use topbar::TopBar;
use views::{DraftsView, ThreadPane, ThreadsView};

#[cfg(feature = "demo")]
use state::show_demo;

#[component]
pub fn App() -> impl IntoView {
    let live = bridge::available();
    let snap = RwSignal::new(empty_snapshot());
    let org = RwSignal::new(storage::get(ORG_KEY).and_then(|v| v.parse().ok()));
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
        pending_files: RwSignal::new(std::collections::HashMap::new()),
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
        sections: Memo::new(move |_| snap.with(|s| folder_list.with(|f| folders::arrange(&s.sections, f)))),
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

    // The unread mark on the tray icon, as in Slack: the backend hears of it only when it changes.
    let badge = Memo::new(move |_| c.snap.with(Snapshot::unread_badge));
    Effect::new(move |_| {
        let (badge, count) = badge.get();
        if bridge::available() {
            spawn_local(async move {
                let _ = bridge::set_unread_badge(badge, count).await;
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
        let popup = c.ctx_menu.with_untracked(Option::is_some)
            || c.picker.get_untracked().is_some()
            || c.lightbox.with_untracked(Option::is_some)
            || c.me_menu.get_untracked();
        if popup {
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
        #[cfg(feature = "demo")]
        show_demo(c);
    }
    // Only these two settings rebuild the whole view; the others are read where they apply.
    let lang_clock = Memo::new(move |_| c.settings.with(|s| (s.lang, s.clock24)));
    let lang = Memo::new(move |_| c.settings.with(|s| s.lang));
    view! {
        {move || {
            // Language and time format before building the view: effects run only after rendering.
            let (lang, clock24) = lang_clock.get();
            set_locale(lang, clock24);
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
            // Recreate the dialog when the language changes, and only then: another preference keeps
            // the focus on its control. The selected section is kept in `pref_tab`.
            crate::i18n::set_current(lang.get());
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

/// A window event listener that lives as long as the component that adds it.
pub(crate) fn scoped_window_listener<E: ev::EventDescriptor + 'static>(event: E, cb: impl Fn(E::EventType) + 'static)
where
    E::EventType: wasm_bindgen::JsCast,
{
    let handle = leptos::__reexports::send_wrapper::SendWrapper::new(window_event_listener(event, cb));
    on_cleanup(move || handle.take().remove());
}
