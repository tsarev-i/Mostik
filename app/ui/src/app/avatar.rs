//! Avatars with presence and status, the hover tip, and what stands for a chat in lists: its
//! person, name and mark.

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};

use super::scoped_window_listener;

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
pub(super) fn presence_class(u: &User) -> &'static str {
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
pub(super) fn status_badge(u: &User) -> Option<AnyView> {
    u.status.as_ref().map(status_view)
}

/// The emoji of status `st` with its text in the hover tip.
pub(super) fn status_view(st: &CustomStatus) -> AnyView {
    view! { <span class="status-emoji" data-tip=status_tip(st)>{status_icon(st, 14)}</span> }.into_any()
}

/// Chat `ch` without what changes with every message (the last time, how far it has been read),
/// for views that show none of it: they are not built again for each message.
pub(super) fn steady_chat(ch: &Chat) -> Chat {
    Chat {
        last_ts: 0,
        peer_seen_ts: 0,
        unread: ch.unread.min(1),
        ..ch.clone()
    }
}

/// A person as an [`Avatar`] draws them, without what changes often and the picture does not
/// show: presence (the dot follows it by itself) and the job title. Views built from it are built
/// again only when the name, photo or status changes: a new `<img>` shows the initials until the
/// photo has loaded again, and the face blinks.
pub(crate) fn face(u: &User) -> User {
    User {
        title: None,
        presence: Presence::Offline,
        availability: Availability::default(),
        ..u.clone()
    }
}

/// The hover tip of a status: its emoji and text. An emoji alone has nothing more to say.
pub(super) fn status_tip(st: &CustomStatus) -> Option<String> {
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

/// A hover tip in place of the native `title`, as in Slack: it appears at once (or after the
/// element's `data-tip-delay` in milliseconds), above an element with `data-tip` (below it near
/// the top of the window) and stays inside the window.
#[component]
pub(super) fn HoverTip() -> impl IntoView {
    #[derive(Clone, PartialEq)]
    struct Tip {
        text: String,
        x: f64,
        y: f64,
        below: bool,
    }
    let tip = RwSignal::new(None::<Tip>);
    // The element the tip is shown for, and the one whose tip waits out its `data-tip-delay` with
    // the timer.
    let owner = StoredValue::new_local(None::<web_sys::Element>);
    let waiting = StoredValue::new_local(None::<(web_sys::Element, TimeoutHandle)>);
    let stop_waiting = move || {
        if let Some((_, timer)) = waiting.try_update_value(Option::take).flatten() {
            timer.clear();
        }
    };
    let hide = move || {
        stop_waiting();
        owner.set_value(None);
        if tip.with_untracked(Option::is_some) {
            tip.set(None);
        }
    };
    let show = move |el: &web_sys::Element| {
        let Some(text) = el.get_attribute("data-tip") else {
            return;
        };
        let r = el.get_bounding_client_rect();
        let below = r.top() < 48.0;
        let next = Tip {
            text,
            x: r.left() + r.width() / 2.0,
            y: if below { r.bottom() + 6.0 } else { r.top() - 6.0 },
            below,
        };
        owner.set_value(Some(el.clone()));
        if tip.with_untracked(|t| t.as_ref() != Some(&next)) {
            tip.set(Some(next));
        }
    };
    scoped_window_listener(ev::mouseover, move |e| {
        let el = e
            .target()
            .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
            .and_then(|el| el.closest("[data-tip]").ok().flatten());
        let Some(el) = el else { return hide() };
        let delay = el.get_attribute("data-tip-delay").and_then(|d| d.parse::<u64>().ok());
        let Some(delay) = delay else {
            stop_waiting();
            return show(&el);
        };
        // Still over the element: its tip is shown or on its way.
        let mine = |w: Option<&web_sys::Element>| w == Some(&el);
        if owner.with_value(|o| mine(o.as_ref())) || waiting.with_value(|w| mine(w.as_ref().map(|(w, _)| w))) {
            return;
        }
        hide();
        let target = el.clone();
        let timer = set_timeout_with_handle(
            move || {
                waiting.set_value(None);
                if target.is_connected() && target.matches(":hover").unwrap_or(false) {
                    show(&target);
                }
            },
            std::time::Duration::from_millis(delay),
        );
        if let Ok(timer) = timer {
            waiting.set_value(Some((el, timer)));
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
pub(super) fn status_icon(st: &CustomStatus, size: u32) -> AnyView {
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
pub(super) fn status_text(st: &CustomStatus) -> String {
    if st.in_meeting {
        tr().in_meeting.to_owned()
    } else {
        st.text.clone()
    }
}

pub(super) fn availability_label(u: &User) -> &'static str {
    match (u.availability, u.presence) {
        (Availability::DoNotDisturb, _) => tr().dnd,
        (Availability::Busy, _) => tr().busy,
        (Availability::Away, _) => tr().away,
        (_, Presence::Online) => tr().active,
        _ => tr().offline_status,
    }
}

/// Photo URL by `AvatarId`: the public file host of the Messenger (`publicAvatarUrl`).
pub(super) fn avatar_url(id: &str, size: u32) -> String {
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
    // The dot follows the person's presence by itself: the avatar is not built again for it.
    let dot = presence.then(|| {
        let (c, id, known) = (use_context::<super::Ctx>(), user.id.clone(), presence_class(&user));
        let class = move || {
            c.and_then(|c| c.snap.with(|s| s.user(&id).map(presence_class)))
                .unwrap_or(known)
        };
        view! { <span class=class></span> }
    });
    // The photo goes over the initials; if it fails to load, it hides and the initials stay. Hidden,
    // not taken out: when the view around is built again, the same `<img>` stays and does not
    // load again, so the photo does not blink.
    let photo = user.avatar.clone().map(|id| {
        let failed = RwSignal::new(false);
        let src = avatar_url(&id, size);
        view! {
            <img class="avatar-img" src=src alt="" style=format!("border-radius:{radius}px")
                hidden=move || failed.get() on:error=move |_| failed.set(true)/>
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

/// The person a chat is with: the peer in a direct chat, you in the chat with yourself.
pub(super) fn chat_person<'a>(s: &'a Snapshot, ch: &Chat) -> Option<&'a User> {
    match &ch.kind {
        ChatKind::Direct { peer } => s.user(peer),
        ChatKind::Notes => s.user(&s.me),
        _ => None,
    }
}

/// Face of a chat: see [`chat_person`].
pub(super) fn chat_face(s: &Snapshot, ch: &Chat) -> Option<User> {
    chat_person(s, ch).cloned()
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
pub(super) struct Mark {
    pub(super) kind: ChatKind,
    pub(super) emoji: Option<String>,
}

impl Mark {
    pub(super) fn of(ch: &Chat) -> Self {
        Mark {
            kind: ch.kind.clone(),
            emoji: ch.icon().map(str::to_owned),
        }
    }
}

pub(super) fn kind_icon(mark: &Mark, size: u32) -> AnyView {
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
}
