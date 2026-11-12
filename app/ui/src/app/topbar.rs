//! The top bar, as in Slack: back and forward, history, search with its results, and "Update".

use std::time::Duration;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::{bridge, ops, time};

use super::avatar::{Avatar, Mark, chat_face, display_title, kind_icon};
use super::dialogs::profile_face;
use super::org::org_name;
use super::scoped_window_listener;
use super::state::ctx;

/// Search result row.
#[derive(Clone)]
enum Hit {
    Chat(ChatId, String, Mark),
    Person(User),
    Message(Box<Message>, String),
}

#[component]
pub(super) fn TopBar() -> impl IntoView {
    let c = ctx();
    let input = NodeRef::<html::Input>::new();
    let open = RwSignal::new(false);
    let history_open = RwSignal::new(false);
    let selected = RwSignal::new(0usize);
    let hits: RwSignal<Vec<Hit>> = RwSignal::new(vec![]);
    let placeholder = move || tr().search_in(&org_name(c));
    // Ctrl+K is the quick switcher, Alt+← / Alt+→ go back and forward, as in Slack. Ctrl+Shift+K
    // is "Add link" in a message field.
    scoped_window_listener(ev::keydown, move |e| {
        if e.ctrl_key() && !e.shift_key() && e.key().eq_ignore_ascii_case("k") {
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
                    <button class="tb-btn" class:active=move || history_open.get() title=tr().history
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
