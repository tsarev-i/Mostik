//! A message in the feed: its row, read marks, reactions, forwarded and replied messages, the reply
//! bar, attachments and the message menu.

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::markup::Me;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::{bridge, ops, time};

use super::avatar::{Avatar, display_title, status_badge};
use super::composer::InlineEditor;
use super::feed::feed_names;
use super::state::{Ctx, CtxMenu, CtxTarget, close_menus, ctx, selected_in_message, wants_native_menu};
use super::text::render_text;

/// Threads are supported in group chats (`0/<ns>/<uuid>`).
fn threads_supported(chat: &ChatId) -> bool {
    chat.0.starts_with("0/")
}

/// Avatars under a message with a thread, as in Slack.
const MAX_THREAD_FACES: usize = 5;

/// A message in the feed. Its parts follow `msg` each on its own, so a change rebuilds only what
/// changed: a new reaction keeps the text selected in the message and an open reaction picker.
#[component]
pub(super) fn MessageRow(
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
        c.picker.set(None);
        c.ctx_menu.set(Some(CtxMenu {
            x: e.client_x() as f64,
            y: e.client_y() as f64,
            target: CtxTarget::User(author_id.clone()),
        }));
    };
    // The full time on hover, as in Slack, and whether an own message has been read: written into
    // `data-tip` as the pointer comes, so the rows do not follow every change of the reads. The tip
    // shows after a second, as Slack's.
    let time_tip = move |e: ev::MouseEvent| {
        if let Some(el) = e
            .current_target()
            .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
        {
            let _ = el.set_attribute("data-tip", &time_tip_text(c, &msg.get_untracked()));
        }
    };
    let gutter = {
        let (show_profile, user_ctx) = (show_profile.clone(), user_ctx.clone());
        move || {
            if grouped.get() {
                return view! { <span class="msg-hover-time" data-tip-delay="1000" on:mouseover=time_tip>{time::hm(ts)}</span> }.into_any();
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
                    <span class="msg-time" data-tip-delay="1000" on:mouseover=time_tip>{time::hm(ts)}</span>
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
                let body = names.with(|n| n.with(|who| render_text(t, who, me, *edited)));
                (!t.is_empty() || *edited).then(|| view! { <div class="msg-text">{body}</div> })
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

/// The hover tip of a message's time, as in Slack: the full date and time and, for an own message
/// someone has read, "(read)" after it. When and by how many it was read does not matter.
fn time_tip_text(c: Ctx, msg: &Message) -> String {
    let full = time::full(msg.ts);
    if read_status(c, msg).is_some_and(|r| r.is_read()) {
        format!("{full} {}", tr().read_mark)
    } else {
        full
    }
}

/// What can be told about the reads of an own message, when the user wants read receipts. The
/// loaded copy is the fresher one: counters change while a menu or a tip is not shown.
fn read_status(c: Ctx, msg: &Message) -> Option<receipts::ReadStatus> {
    if !c.settings.with_untracked(|s| s.show_read_receipts) {
        return None;
    }
    c.snap.with_untracked(|s| {
        let m = s
            .messages
            .iter()
            .find(|m| m.chat_id == msg.chat_id && m.ts == msg.ts)
            .unwrap_or(msg);
        receipts::read_status(s.chat(&msg.chat_id)?, &s.me, m)
    })
}

/// Who read a message, for its menu: "Anna, Boris, Vera and 2 more".
fn readers(s: &Snapshot, count: u32, recent: &[UserId]) -> String {
    let names: Vec<String> = recent
        .iter()
        .filter_map(|id| s.user(id).filter(|u| u.has_name()).map(|u| u.display_name.clone()))
        .collect();
    tr().readers(&names, count.saturating_sub(names.len() as u32))
}

/// What the message menu says about the reads of an own message, if anything.
fn read_info(c: Ctx, msg: &Message) -> Option<String> {
    Some(match read_status(c, msg)? {
        receipts::ReadStatus::Seen { seen: false, .. } => tr().not_seen_yet.to_owned(),
        receipts::ReadStatus::Seen { .. } => tr().read.to_owned(),
        receipts::ReadStatus::Read { count: 0, .. } => tr().not_read_yet.to_owned(),
        receipts::ReadStatus::Read {
            count,
            recent,
            everyone,
        } => {
            let head = tr().read_by(count, everyone);
            let names = c.snap.with_untracked(|s| readers(s, count, &recent));
            if names.is_empty() {
                head
            } else {
                format!("{head}: {names}")
            }
        }
    })
}

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

/// Forwarded message, like a "shared message" in Slack: a bar on the left, author, text and origin.
#[component]
pub(super) fn ForwardedBlock(f: Forwarded) -> impl IntoView {
    let c = ctx();
    let (author, me_name, me_id, chat) = c.snap.with_untracked(|s| {
        (
            s.user(&f.author).cloned(),
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
    let face = author.unwrap_or_else(|| User::named(f.author.clone(), name.clone()));
    // Like the message's own text, it follows the names it mentions and the emoji set.
    let names = feed_names(c);
    let text = (!f.text.is_empty()).then(|| {
        let text = f.text.clone();
        move || {
            let me = Me {
                id: &me_id.0,
                name: &me_name,
            };
            names.with(|n| n.with(|who| render_text(&text, who, me, false)))
        }
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
            {text.map(|text| view! { <div class="msg-text">{text}</div> })}
            <Attachments list=f.attachments.clone()/>
            {footer}
        </div>
    }
}

/// A message in one line of a list: its text without markup, else the text of the message it
/// forwards (`↪ …`), else 📎 for attachments alone.
pub(super) fn message_preview(s: &Snapshot, m: &Message) -> String {
    let text = s.plain(&m.text);
    match m.forwarded.first() {
        _ if !text.is_empty() => text,
        Some(f) => format!("↪ {}", s.plain(&f.text)),
        None if !m.attachments.is_empty() => "📎".to_owned(),
        None => text,
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
pub(super) fn ReplyBar(target: ops::ReplyTarget) -> impl IntoView {
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

pub(super) fn human_size(bytes: u64) -> String {
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
            <div class="images">
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
                            Err(e) => ops::failed(c, tr().err_download, e),
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
pub(super) fn message_menu_items(c: Ctx, msg: Message, mine: bool, quote: Option<String>) -> impl IntoView {
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
