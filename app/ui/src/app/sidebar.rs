//! The sidebar: "Find a conversation", the call bar, the sections with their chats, and dragging
//! chats between them.

use std::time::Duration;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::settings::{SidebarFilter, SidebarSort};
use crate::{folders, hidden, ops, order};

use super::avatar::{Avatar, Mark, chat_face, chat_person, display_title, kind_icon, status_badge};
use super::feed::typers;
use super::menus::open_menu_below;
use super::org::{in_current_org, org_name};
use super::state::{ChatDrag, Ctx, CtxMenu, CtxTarget, DropAt, NewChannel, ViewMode, ctx};
use super::views::{ActivityList, DmList};

#[component]
pub(super) fn Sidebar() -> impl IntoView {
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
            let now = ops::now_mcs();
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
pub(super) fn chat_drag_to(c: Ctx, e: &ev::MouseEvent) {
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
pub(super) fn chat_drop(c: Ctx) {
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
                s.chat(&id)
                    .map(|ch| (ch.clone(), display_title(s, ch), chat_face(s, ch)))
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
                let user = chat_person(s, &ch)?;
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
