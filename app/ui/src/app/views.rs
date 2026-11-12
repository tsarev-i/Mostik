//! The thread pane and the pages in place of a chat: direct messages, mentions, threads, drafts and
//! sent.

use std::collections::HashSet;

use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::markup::Me;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::{bridge, ops, time};

use super::avatar::{Avatar, Mark, chat_face, display_title, kind_icon, status_badge};
use super::composer::Composer;
use super::feed::{Feed, call_title, chat_note_text};
use super::message::{MessageRow, message_preview};
use super::state::{CtxMenu, CtxTarget, ViewMode, ctx};
use super::text::render_text;

#[component]
pub(super) fn ThreadPane() -> impl IntoView {
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

/// The "DMs" view in the rail: all direct conversations with their last message, like DMs in Slack.
#[component]
pub(super) fn DmList() -> impl IntoView {
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
                    let text = c.snap.with(|s| message_preview(s, m));
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
pub(super) fn ActivityList() -> impl IntoView {
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
            <button class="act-tab" class:active=move || tab.get() == t on:click=move |_| tab.set(t)>{label}</button>
        }
    };
    view! {
        <div class="dm-list">
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
pub(super) fn ThreadsView() -> impl IntoView {
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
                    let text = names.with(|who| render_text(&t.parent.text, who, Me { id: &me_id.0, name: &me_name }, false));
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
                            <span>{match author {
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
pub(super) fn DraftsView() -> impl IntoView {
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
                <button class="tab" class:active=move || tab.get() == DraftsTab::Drafts on:click=move |_| tab.set(DraftsTab::Drafts)>
                    <Icon icon=I::Compose size=14/><span>{tr().drafts}</span>
                    {move || { let n = drafts.with(Vec::len); (n > 0).then(|| view! { <span class="tab-count">{n}</span> }) }}
                </button>
                <button class="tab" class:active=move || tab.get() == DraftsTab::Sent on:click=move |_| tab.set(DraftsTab::Sent)>
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
                                <span>{match face {
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
                        let text = c.snap.with(|s| message_preview(s, &m));
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
