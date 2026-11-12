//! Right-click and drop-down menus, as in Slack: a person, a section, the organization, the
//! sidebar settings and a sidebar chat, with their submenus.

use std::collections::HashSet;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::settings::{Settings, SidebarFilter, SidebarSort};
use crate::{folders, ops, storage};

use super::avatar::{avatar_url, display_title};
use super::message::message_menu_items;
use super::org::{ORG_KEY, current_org, org_avatar, org_name};
use super::state::{Ctx, CtxMenu, CtxTarget, DetailsTab, FolderEdit, NewChannel, close_menus, ctx};

/// Right-click menu. Opens at the cursor and never leaves the window.
#[component]
pub(super) fn ContextMenu(menu: CtxMenu) -> impl IntoView {
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

/// "Filter conversations by" as menu items.
fn filter_items(c: Ctx) -> AnyView {
    SidebarFilter::ALL
        .into_iter()
        .map(|f| {
            radio_item(
                c,
                tr().sidebar_filter(f),
                move |s| s.sidebar_filter == f,
                move |s| s.sidebar_filter = f,
            )
        })
        .collect_view()
        .into_any()
}

/// "Sort conversations by" as menu items.
fn sort_items(c: Ctx) -> AnyView {
    SidebarSort::ALL
        .into_iter()
        .map(|o| {
            radio_item(
                c,
                tr().sidebar_sort(o),
                move |s| s.sidebar_sort == o,
                move |s| s.sidebar_sort = o,
            )
        })
        .collect_view()
        .into_any()
}

fn filter_submenu(c: Ctx) -> impl IntoView {
    choice_submenu(
        tr().filter,
        Signal::derive(move || tr().sidebar_filter(c.settings.with(|s| s.sidebar_filter))),
        move || filter_items(c),
    )
}

fn sort_submenu(c: Ctx) -> impl IntoView {
    choice_submenu(
        tr().sort,
        Signal::derive(move || tr().sidebar_sort(c.settings.with(|s| s.sidebar_sort))),
        move || sort_items(c),
    )
}

/// Opens a menu at the bottom left corner of the clicked element.
pub(super) fn open_menu_below(c: Ctx, e: &ev::MouseEvent, target: CtxTarget) {
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
        <div class="menu-head">
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
        return view! { <button class="menu-radio" disabled=true><span class="menu-tick">"✓"</span>{name}</button> }
            .into_any();
    }
    list.into_iter()
        .map(|o| {
            let checked = Some(o.id) == current;
            let id = o.id;
            view! {
                <button class="menu-radio" on:click=move |_| {
                    close_menus(c);
                    c.org.set(Some(id));
                    storage::set(ORG_KEY, &id.to_string());
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
    view! {
        <div class="me-sub" on:mouseenter=move |_| sub_open.set(true) on:mouseleave=move |_| sub_open.set(false)>
            <button class="me-sub-btn" on:click=move |_| sub_open.set(true)>
                <span>{tr().filter_and_sort}</span><Icon icon=I::ChevronRight size=14/>
            </button>
            <Show when=move || sub_open.get()>
                <div class="me-submenu">
                    <div class="menu-label">{tr().filter_by}</div>
                    {filter_items(c)}
                    <div class="menu-sep"></div>
                    <div class="menu-label">{tr().sort_by}</div>
                    {sort_items(c)}
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
