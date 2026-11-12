//! The chat pane: its header, the message list and pinned messages, and the feed with its call,
//! note and day lines.

use leptos::context::Provider;
use leptos::html;
use leptos::prelude::*;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::{ops, time};

use super::avatar::{Avatar, Mark, chat_face, display_title, kind_icon, status_badge};
use super::menus::open_menu_below_end;
use super::message::MessageRow;
use super::state::{Ctx, CtxMenu, CtxTarget, DetailsTab, close_menus, ctx};

#[component]
pub(super) fn ChannelHeader() -> impl IntoView {
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
                        // There is no one to call in the chat with oneself.
                        if info.with(|i| i.as_ref().is_some_and(|(ch, _, _)| matches!(ch.kind, ChatKind::Notes))) {
                            return None;
                        }
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
                        Some(view! {
                            <button class="huddle-btn" class:live=live class:active=joined
                                title=title
                                on:click=move |_| ops::start_call(c, c.current.get_untracked())>
                                {if live {
                                    view! { <Icon icon=I::Headphones size=20/> }.into_any()
                                } else {
                                    view! { <Icon icon=I::Video size=20/> }.into_any()
                                }}
                                <span>{if live { tr().call_join } else { tr().call }}</span>
                            </button>
                        })
                    }}
                    // "⋮" with the channel's menu, as in Slack. A second click closes it.
                    {move || info.get().filter(|(ch, _, _)| !matches!(ch.kind, ChatKind::Direct { .. } | ChatKind::Notes)).map(|(ch, _, _)| {
                        let open = move || c.ctx_menu.with(|m| matches!(m, Some(CtxMenu { target: CtxTarget::ChatHeader(_), .. })));
                        view! {
                            <button class="icon-btn ch-more" class:open=open title=tr().more
                                on:mousedown=|e| e.stop_propagation()
                                on:click=move |e| {
                                    if open() {
                                        close_menus(c);
                                    } else {
                                        open_menu_below_end(c, &e, CtxTarget::ChatHeader(ch.id.clone()));
                                    }
                                }>
                                <Icon icon=I::Kebab size=20/>
                            </button>
                        }
                    })}
                </div>
            </div>
            <nav class="ch-tabs">
                <button class="tab" class:active=move || !c.pins_tab.get() on:click=move |_| c.pins_tab.set(false)>
                    <Icon icon=I::Chat size=14/><span>{tr().tab_messages}</span>
                </button>
                // Like the "Pins" tab in Slack: only if the chat has a pinned message.
                {move || info.get().and_then(|(ch, _, _)| ch.pinned).map(|_| view! {
                    <button class="tab" class:active=move || c.pins_tab.get() on:click=move |_| c.pins_tab.set(true)>
                        <Icon icon=I::Pin size=14/><span>{tr().tab_pins}</span>
                    </button>
                })}
            </nav>
        </header>
    }
}

/// The "X is typing…" line for a chat.
pub(super) fn typing_line(c: Ctx, chat: &ChatId) -> Option<String> {
    tr().typing(&typers(c, chat))
}

/// Names of the people typing in a chat now.
pub(super) fn typers(c: Ctx, chat: &ChatId) -> Vec<String> {
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
pub(super) fn MessageList() -> impl IntoView {
    let c = ctx();
    let list_ref = NodeRef::<html::Div>::new();
    let messages = Memo::new(move |_| {
        let id = c.current.get();
        c.snap.with(|s| s.channel_messages(&id))
    });
    // Scrolling: to the bottom when the chat changes or new messages arrive, if the user is at the bottom;
    // when older messages load, keep the visible position.
    let stick = RwSignal::new(true);
    let anchor: RwSignal<Option<(i32, i32)>> = RwSignal::new(None);
    // The last scroll position, to tell the user scrolling up from the list moving by itself.
    let last_top = StoredValue::new(None::<i32>);
    Effect::new(move |prev: Option<ChatId>| {
        let id = c.current.get();
        if prev.as_ref() != Some(&id) {
            stick.set(true);
            anchor.set(None);
            last_top.set_value(None);
        }
        id
    });
    Effect::new(move |_| {
        messages.track();
        // An older page has come, or failed, when the loading stops.
        c.loading_older.track();
        // The list may be gone by the next frame (another view opened).
        request_animation_frame(move || {
            let Some(el) = list_ref.try_get_untracked().flatten() else {
                return;
            };
            match anchor.try_get_untracked().flatten() {
                Some(_) if c.loading_older.get_untracked() => {}
                Some((height, top)) => {
                    el.set_scroll_top(el.scroll_height() - height + top);
                    anchor.try_set(None);
                }
                None if stick.try_get_untracked() == Some(true) => el.set_scroll_top(el.scroll_height()),
                None => {}
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
        let top = el.scroll_top();
        stick.set(el.scroll_height() - top - el.client_height() < 120);
        let up = last_top.get_value().is_some_and(|last| top < last);
        last_top.set_value(Some(top));
        // Older messages load as the user scrolls up near the top. Not when the list moves by itself:
        // another chat is shorter, it goes to the bottom, or, before the last older page is in place,
        // the spinner goes away.
        if up && top < 200 && anchor.get_untracked().is_none() && ops::load_older(c) {
            anchor.set(Some((el.scroll_height(), top)));
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
            <Feed messages=messages/>
        </div>
    }
}

#[component]
pub(super) fn PinsView() -> impl IntoView {
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

/// What the lines of a feed share: the names that mentions show.
#[derive(Clone, Copy)]
struct FeedCtx {
    names: Memo<mostik_core::markup::MentionNames>,
}

/// Mention names for a message line: those of its feed; a line outside a feed (a pinned message,
/// the parent of a thread) follows the snapshot itself.
pub(super) fn feed_names(c: Ctx) -> Memo<mostik_core::markup::MentionNames> {
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
pub(super) fn Feed(messages: Memo<Vec<Message>>, #[prop(optional)] in_thread: bool) -> impl IntoView {
    let c = ctx();
    // For the lines only, through an owner of their own: what replaces the feed (a pinned message
    // shown in its place, the parent of the next thread) must not find it once it is disposed.
    let shared = FeedCtx {
        names: Memo::new(move |_| c.snap.with(Snapshot::mention_names)),
    };
    let days = Memo::new(move |_| messages.with(|list| feed::layout(list, time::day_key)));
    let line = move |key: (ChatId, Timestamp)| view! { <FeedRow days=days ts=key.1 in_thread=in_thread/> };
    let key_of = |l: &feed::FeedLine| (l.msg.chat_id.clone(), l.msg.ts);
    if in_thread {
        let keys = Memo::new(move |_| days.with(|d| d.iter().flat_map(|d| &d.lines).map(key_of).collect::<Vec<_>>()));
        return view! {
            <Provider value=shared>
                <For each=move || keys.get() key=|k| k.clone() children=line/>
            </Provider>
        }
        .into_any();
    }
    let day_keys = Memo::new(move |_| {
        days.with(|d| {
            d.iter()
                .map(|d| (d.lines[0].msg.chat_id.clone(), d.day, d.lines[0].msg.ts))
                .collect::<Vec<FeedDayKey>>()
        })
    });
    view! {
        <Provider value=shared>
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
        </Provider>
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
pub(super) fn call_title(anchor: &CallNote, end: Option<&Message>, me: &UserId) -> (&'static str, bool) {
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
            <span class="call-note-icon"><span class="call-note-badge"><Icon icon=icon size=20/></span></span>
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
pub(super) fn chat_note_text(s: &Snapshot, m: &Message) -> Option<String> {
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
        <div class="msg call-note ended" data-ts=msg.ts.to_string()>
            <span class="call-note-icon"><span class="call-note-badge"><Icon icon=icon size=20/></span></span>
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
