//! Dialogs: the people picker, browsing, forwarding, a new message, creating a channel, channel
//! details and adding people, the confirmations, the section name and "About".

use std::time::Duration;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::modal::Modal;
use crate::{bridge, folders, ops};

use super::avatar::{Avatar, Mark, chat_face, display_title, kind_icon};
use super::message::{ForwardedBlock, delete_preview};
use super::org::in_current_org;
use super::scoped_window_listener;
use super::state::{DetailsTab, FolderEdit, NewChannel, ViewMode, ctx};

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
                        Err(e) => ops::failed(c, tr().err_search, e),
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
                            let face = profile_face(&p);
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

/// A person from the directory, for an avatar.
pub(super) fn profile_face(p: &UserProfile) -> User {
    User {
        title: p.position.clone(),
        avatar: p.avatar.clone(),
        ..User::named(p.id.clone(), p.display_name.clone())
    }
}

/// "Browse channels", like in Slack: your channels and recommended public groups.
#[component]
pub(super) fn BrowseChannelsDialog() -> impl IntoView {
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
                        c.snap.update(|s| ops::add_channel(s, chat));
                        close();
                        ops::open_chat(c, cid);
                    }
                    Err(e) => ops::failed(c, tr().err_join, e),
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
        <Modal class="browse-modal" on_close=Callback::new(move |_| close())>
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
        </Modal>
    }
}

/// "Forward message", like Forward in Slack: where to, a comment and a preview.
#[component]
pub(super) fn ForwardDialog(msg: Message) -> impl IntoView {
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
                Err(e) => ops::failed(c, tr().err_forward, e),
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
        <Modal class="forward-modal" on_close=Callback::new(move |_| close())>
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
        </Modal>
    }
}

/// "New message" (the pencil in the sidebar header): pick a person to open or create a direct chat.
#[component]
pub(super) fn NewMessageDialog() -> impl IntoView {
    let c = ctx();
    let close = move || c.new_message.set(false);
    let me = c.snap.with_untracked(|s| s.me.clone());
    let exclude = Signal::derive(move || vec![me.clone()]);
    view! {
        <Modal on_close=Callback::new(move |_| close())>
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
        </Modal>
    }
}

/// "Create a channel", as in Slack: step 1 is the name and description, step 2 is the people.
#[component]
pub(super) fn CreateChannelDialog(to: NewChannel) -> impl IntoView {
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
                    c.snap.update(|s| ops::add_channel(s, chat));
                    if let Some(folder) = folder {
                        ops::change_folders(c, |f| folders::move_chat(f, &id, Some(&folder), None));
                    }
                    close();
                    c.view.set(ViewMode::Home);
                    ops::open_chat(c, id);
                }
                Err(e) => ops::failed(c, tr().err_create_channel, e),
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
        <Modal on_close=Callback::new(move |_| close())>
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
        </Modal>
    }
}

/// Channel details, as in Slack: "About" (name, description, leave) and "Members".
#[component]
pub(super) fn ChannelDetailsDialog(initial: DetailsTab) -> impl IntoView {
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
                        Err(e) => return ops::failed(c, tr().err_update_chat, e),
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
                    Err(e) => ops::failed(c, tr().err_members, e),
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
        <Modal class="details" on_close=Callback::new(move |_| close())>
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
                <button class="tab" class:active=move || tab.get() == DetailsTab::About on:click=move |_| tab.set(DetailsTab::About)>{tr().about_channel}</button>
                <button class="tab" class:active=move || tab.get() == DetailsTab::Members on:click=move |_| tab.set(DetailsTab::Members)>
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
        </Modal>
        {move || adding.get().then(|| {
            let add = change_add.clone();
            view! {
                <AddPeopleDialog chat_id=picker_chat.clone() title=name.get_untracked() exclude=exclude
                    on_close=Callback::new(move |_| adding.set(false))
                    on_add=Callback::new(move |ids: Vec<UserId>| { adding.set(false); add(ids, true); })/>
            }
        })}
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
        <Modal nested=true on_close=on_close>
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
        </Modal>
    }
}

#[component]
pub(super) fn DeleteDialog() -> impl IntoView {
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
            .and_then(|m| c.snap.with(|s| delete_preview(s, &m)))
            .map(|line| view! { <blockquote class="quote">{line}</blockquote> })
    };
    view! {
        <Modal on_close=Callback::new(move |_| close())>
            <header><h2>{tr().delete_message}</h2></header>
            <p>{tr().delete_message_warning}</p>
            {preview}
            <div class="modal-actions">
                <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                <button class="btn danger" on:click=confirm>{tr().delete}</button>
            </div>
        </Modal>
    }
}

/// Leave-channel confirmation opened from the sidebar menu.
#[component]
pub(super) fn LeaveDialog(id: ChatId) -> impl IntoView {
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
        <Modal on_close=Callback::new(move |_| close())>
            <header><h2>{tr().leave_title(&title)}</h2></header>
            <p>{tr().leave_confirm}</p>
            <div class="modal-actions">
                <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                <button class="btn danger" on:click=confirm>{tr().leave_channel}</button>
            </div>
        </Modal>
    }
}

/// "New section" / "Rename section": the name of an own sidebar section.
#[component]
pub(super) fn FolderDialog(edit: FolderEdit) -> impl IntoView {
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
        <Modal on_close=Callback::new(move |_| close())>
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
        </Modal>
    }
}

#[component]
pub(super) fn AboutDialog() -> impl IntoView {
    let c = ctx();
    view! {
        <Modal on_close=Callback::new(move |_| c.about.set(false))>
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
        </Modal>
    }
}
