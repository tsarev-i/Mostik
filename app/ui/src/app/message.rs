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
    /// A copy outside the loaded history (an old pinned message): it is only shown. What changes a
    /// message (reactions, the menu, its thread) works on the loaded ones, so it is not offered.
    #[prop(optional)]
    detached: bool,
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
    // Only what the line shows of the author: their presence changes as they write, and the
    // avatar would be built again and blink.
    let author = {
        let id = author_id.clone();
        Memo::new(move |_| c.snap.with(|s| s.user(&id).map(super::avatar::face)))
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
    let media = Memo::new(move |_| msg.with(|m| m.media.clone()));
    let reactions = Memo::new(move |_| msg.with(|m| m.reactions.clone()));
    // What the user may do in the chat, as far as known: a channel they only read takes no
    // reactions, say. It may change while the message shows.
    let role = {
        let id = chat_id.clone();
        Memo::new(move |_| c.snap.with(|s| s.chat(&id).and_then(|ch| ch.role)))
    };
    let can = move |right: Right| role.get().is_none_or(|r| r.can(right));
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
            // A thread's parent opens the editor where its edit was opened: in the chat or the pane.
            c.editing.with(|e| {
                e.as_ref().is_some_and(|m| {
                    (m.chat_id.clone(), m.ts) == key
                        && (m.thread_parent.is_some() || c.editing_in_thread.get() == in_thread)
                })
            })
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
    // shows after half a second.
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
                return view! { <span class="msg-hover-time" data-tip-delay="500" on:mouseover=time_tip>{time::hm(ts)}</span> }.into_any();
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
                    <span class="msg-time" data-tip-delay="500" on:mouseover=time_tip>{time::hm(ts)}</span>
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
                                // The button's tooltip says who reacted: the image has none to cover it.
                                Some(e) => crate::custom::image_untitled(&e, "custom-emoji").into_any(),
                                None if mostik_core::custom_emoji::is_custom_reaction(kind) => {
                                    view! { <span class="emoji">"▫️"</span> }.into_any()
                                }
                                None => view! { <span class="emoji">{glyph.clone()}</span> }.into_any(),
                            };
                            view! {
                                <button class=if r.mine { "reaction mine" } else { "reaction" }
                                    title=title disabled=detached || !can(Right::SendReactions)
                                    on:click=move |_| msg.with_untracked(|m| ops::toggle_reaction(c, m, kind))>
                                    {glyph}
                                    <span class="count">{r.count}</span>
                                </button>
                            }
                        }).collect_view()}
                        {(!detached && can(Right::SendReactions)).then(|| view! {
                            <button class="reaction add" title=tr().add_reaction on:click=move |e| { c.picker_at.set(crate::emoji::anchor_of(&e)); c.picker.set(Some(key.clone())) }>
                                <Icon icon=I::SmilePlus size=16/>
                            </button>
                        })}
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
                    <button class="thread-summary" disabled=detached on:click=open_thread>
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
        view! {
            <Show when=move || can(Right::WriteToThread)>
                <button title=tr().reply_in_thread on:click=open_thread><Icon icon=I::Threads size=16/></button>
            </Show>
        }
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
        if detached || editing.get_untracked() || (quote.is_none() && wants_native_menu(&e)) {
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
                in_thread,
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
                    {move || media.get().map(|m| view! { <MediaView media=m/> })}
                    {forwarded_blocks}
                    {move || view! { <Attachments list=attachments.get()/> }}
                    {reactions_view}
                    {thread_view}
                </div>
            </div>
            {(!detached).then(|| view! {
                <div class="msg-actions">
                    <Show when=move || can(Right::SendReactions)>
                        <button title=tr().reaction on:click={
                            let key = key.clone();
                            move |e| { c.picker_at.set(crate::emoji::anchor_of(&e)); c.picker.set(Some(key.clone())) }
                        }><Icon icon=I::SmilePlus size=16/></button>
                    </Show>
                    {reply_btn}
                    <Show when=move || can(Right::MarkAsImportant)>
                        <button title=move || if important.get() { tr().unmark_important } else { tr().mark_important }
                            on:click=move |_| msg.with_untracked(|m| ops::toggle_important(c, m))>
                            <Icon icon=I::Bookmark size=16/>
                        </button>
                    </Show>
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
                        c.ctx_menu.set(Some(CtxMenu { x, y, target: CtxTarget::Message { msg: Box::new(msg.get_untracked()), mine, quote, in_thread } }));
                    }><Icon icon=I::Kebab size=16/></button>
                </div>
            })}
            <Show when=move || picker_open.get()>
                <ReactionPicker msg=msg/>
            </Show>
        </div>
    }
}

/// The hover tip of a message's time, as in Slack: the full date and time and, for an own message
/// someone has read, "(read)" after it in a direct chat. In a group, how many of the members have
/// read it and the latest of them by name, as the reaction tip names people (#49): in a chat of
/// 200, one reader and everyone must not look the same. Telemost names the latest three only.
fn time_tip_text(c: Ctx, msg: &Message) -> String {
    let full = time::full(msg.ts);
    match read_status(c, msg) {
        Some(receipts::ReadStatus::Read {
            count,
            recent,
            everyone,
        }) if count > 0 => {
            let (others, names) = c.snap.with_untracked(|s| {
                // The members besides the author; unknown while the chat has not said.
                let others = s
                    .chat(&msg.chat_id)
                    .map(|ch| ch.member_count.saturating_sub(1))
                    .filter(|n| *n > 0);
                let names: Vec<String> = recent
                    .iter()
                    .filter(|u| **u != s.me)
                    .filter_map(|u| s.user(u).filter(|u| u.has_name()).map(|u| u.display_name.clone()))
                    .collect();
                (others, names)
            });
            let mut tip = format!("{full}\n{}", tr().read_by(count, others, everyone));
            if !names.is_empty() {
                let more = count.saturating_sub(names.len() as u32);
                tip.push('\n');
                tip.push_str(&tr().readers(&names, more));
            }
            tip
        }
        Some(r) if r.is_read() => format!("{full} {}", tr().read_mark),
        _ => full,
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
            None if mostik_core::custom_emoji::is_custom_reaction(r.kind) => tr().unknown_custom_emoji.to_owned(),
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
    let author_id = f.author.clone();
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
            <button class="fwd-head person-link" on:click=move |_| ops::open_profile(c, author_id.clone())>
                <Avatar user=face size=20/><b>{name}</b>
            </button>
            {text.map(|text| view! { <div class="msg-text">{text}</div> })}
            {f.media.clone().map(|m| view! { <MediaView media=m/> })}
            <Attachments list=f.attachments.clone()/>
            {footer}
        </div>
    }
}

/// What a message is besides text and attachments, in the feed: a voice message, a sticker.
#[component]
fn MediaView(media: Media) -> impl IntoView {
    match media {
        Media::Voice(v) => view! { <super::voice::VoiceCard voice=v/> }.into_any(),
        Media::Sticker(s) => view! { <StickerView sticker=s/> }.into_any(),
    }
}

/// A sticker: its picture from Telemost's public host, unless pictures are off in the feed (then
/// the word). The browser demo has no pictures and shows a stand-in.
#[component]
fn StickerView(sticker: Sticker) -> impl IntoView {
    let c = ctx();
    let inline = c.settings.with_untracked(|s| s.inline_media);
    match sticker.url("middle") {
        Some(src) if c.live && inline => {
            view! { <img class="sticker" src=src alt=tr().sticker title=tr().sticker loading="lazy"/> }.into_any()
        }
        _ if !c.live => view! { <div class="sticker stand-in" title=tr().sticker>"🐱"</div> }.into_any(),
        _ => view! { <div class="msg-text sticker-word">{format!("🖼 {}", tr().sticker)}</div> }.into_any(),
    }
}

/// [`Media`] in one line of a list.
fn media_line(media: &Media) -> String {
    match media {
        Media::Voice(v) => super::voice::voice_line(v),
        Media::Sticker(_) => tr().sticker.to_owned(),
    }
}

/// A message in one line of a list: its text without markup, else what it is (a voice message),
/// else the text of the message it forwards or what that one is (`↪ …`), else 📎 for attachments
/// alone.
pub(super) fn message_preview(s: &Snapshot, m: &Message) -> String {
    let text = s.plain(&m.text);
    match m.forwarded.first() {
        _ if !text.is_empty() => text,
        _ if m.media.is_some() => m.media.as_ref().map(media_line).unwrap_or_default(),
        Some(f) => {
            let text = s.plain(&f.text);
            match &f.media {
                Some(media) if text.is_empty() => format!("↪ {}", media_line(media)),
                _ => format!("↪ {text}"),
            }
        }
        None if !m.attachments.is_empty() => "📎".to_owned(),
        None => text,
    }
}

/// The delete dialog's quote, so the user sees which message goes: its text without markup, else
/// its first attachment, else what it forwards (`↪ …`); `None` when there is nothing to show.
pub(super) fn delete_preview(s: &Snapshot, m: &Message) -> Option<String> {
    let text = s.plain(&m.text);
    let line = if !text.trim().is_empty() {
        text
    } else if let Some(media) = &m.media {
        media_line(media)
    } else if let Some(a) = attachment_line(&m.attachments) {
        a
    } else {
        let f = m.forwarded.first()?;
        let text = s.plain(&f.text);
        let what = if !text.trim().is_empty() {
            text
        } else if let Some(media) = &f.media {
            media_line(media)
        } else {
            attachment_line(&f.attachments)?
        };
        format!("↪ {what}")
    };
    Some(line.chars().take(200).collect())
}

/// The first attachment in one line: 🖼 or 📄 and its name.
fn attachment_line(attachments: &[Attachment]) -> Option<String> {
    attachments.first().map(|a| match a.kind {
        AttachmentKind::Image => format!("🖼 {}", a.name),
        AttachmentKind::File => format!("📄 {}", a.name),
    })
}

/// One line about the message a reply answers: the quoted fragment, else its text without markup,
/// else what it is (a voice message), else its first attachment; a deleted message has none of them.
fn reply_snippet(
    s: &Snapshot,
    quote: Option<&str>,
    text: &str,
    media: Option<&Media>,
    attachments: &[Attachment],
) -> String {
    let line = match quote.filter(|q| !q.trim().is_empty()) {
        Some(q) => q.to_owned(),
        None if !text.trim().is_empty() => s.mention_names().with(|who| mostik_core::markup::plain(text, who)),
        None if media.is_some() => media.map(media_line).unwrap_or_default(),
        None => attachment_line(attachments).unwrap_or_else(|| tr().reply_deleted.to_owned()),
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
            reply_snippet(s, f.quote.as_deref(), &f.text, f.media.as_ref(), &f.attachments),
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
    let key = super::drafts::DraftKey::field(target.msg.chat_id.clone(), target.msg.thread_parent);
    let m = &target.msg;
    let (name, line) = c.snap.with_untracked(|s| {
        (
            reply_author(s, &m.author, ""),
            reply_snippet(s, target.quote.as_deref(), &m.text, m.media.as_ref(), &m.attachments),
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
            <button class="icon-btn" title=tr().cancel_reply on:click=move |_| super::drafts::set_reply(c, &key, None)>
                <Icon icon=I::X size=14/>
            </button>
        </div>
    }
}

/// Saves attachment `id` named `name` to "Downloads" and shows it in its folder, as a click on its
/// card does; the toast says where it went.
pub(super) fn save_attachment(c: Ctx, id: String, name: String) {
    spawn_local(async move {
        match bridge::save_file(&id, &name).await {
            Ok(path) => c.notice.set(Some(format!("{}: {path}", tr().saved_to))),
            Err(e) => ops::failed(c, tr().err_download, e),
        }
    });
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
                let download = move |_| save_attachment(c, id.clone(), name.clone());
                view! {
                    <button class="file-card" title=tr().download on:click=download>
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
pub(super) fn message_menu_items(
    c: Ctx,
    msg: Message,
    mine: bool,
    quote: Option<String>,
    in_pane: bool,
) -> impl IntoView {
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
    // A reply goes to the field it is opened in: the thread's in the pane.
    let (may_reply, may_edit) = c.snap.with_untracked(|s| {
        s.chat(&msg.chat_id)
            .map_or((true, true), |ch| (ch.can_post(in_pane), ch.can(Right::EditMessage)))
    });
    let copy = |text: String| {
        let _ = window().navigator().clipboard().write_text(&text);
    };
    // Replies go to messages, not to notes; as in Telemost, they lead the menu.
    let replies = (!msg.is_note() && may_reply).then(|| {
        let (m_reply, m_quote) = (msg.clone(), msg.clone());
        let selected = quote.clone();
        view! {
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); ops::start_reply(c, m_reply.clone(), None); }>
                {menu_item(I::Reply, tr().reply)}
            </button>
            {selected.map(|q| view! {
                <button class="menu-icon-item" on:click=move |_| { close_menus(c); ops::start_reply(c, m_quote.clone(), Some(q.clone())); }>
                    {menu_item(I::Quote, tr().quote_selection)}
                </button>
            })}
            <div class="menu-sep"></div>
        }
    });
    // Who read an own group message, as far as Telemost tells it.
    let audience = matches!(read_status(c, &msg), Some(receipts::ReadStatus::Read { .. })).then(|| {
        let m = msg.clone();
        view! {
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); c.audience.set(Some(m.clone())); }>
                {menu_item(I::Check, tr().who_read)}
            </button>
        }
    });
    view! {
        {replies}
        {audience}
        // Telemost does not make your own message unread, so the item is only for others' messages.
        {(!in_thread && !mine).then(|| view! {
            <button class="menu-icon-item" on:click=move |_| { close_menus(c); ops::mark_unread(c, &m_unread); }>
                {menu_item(I::MarkUnread, tr().mark_unread)}
            </button>
        })}
        <button class="menu-icon-item" on:click=move |_| { close_menus(c); copy(ops::message_link(&m_link)); c.notice.set(Some(tr().link_copied.into())); }>
            {menu_item(I::Link, tr().copy_link)}
        </button>
        // With text of the message selected, the selection, as "Copy" does anywhere else.
        <button class="menu-icon-item" on:click=move |_| {
            close_menus(c);
            copy(quote.clone().unwrap_or_else(|| match msg.media.as_ref().and_then(Media::voice) {
                // A voice message's text is what Telemost made of the speech.
                Some(v) if msg.text.is_empty() => v.text.clone(),
                _ => c.snap.with_untracked(|s| s.mention_names().with(|who| mostik_core::markup::ids_to_names(&msg.text, who).0)),
            }));
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
            {may_edit.then(|| view! {
                <button class="menu-icon-item" on:click=move |_| {
                    close_menus(c);
                    c.editing_in_thread.set(in_pane);
                    c.editing.set(Some(m_edit.clone()));
                }>{menu_item(I::Pencil, tr().edit)}</button>
            })}
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

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str, attachments: Vec<Attachment>, forwarded: Vec<Forwarded>) -> Message {
        Message {
            chat_id: ChatId("general".into()),
            ts: 1,
            author: UserId("anna".into()),
            text: text.into(),
            thread_parent: None,
            edited: false,
            important: false,
            reactions: vec![],
            thread: None,
            seq_no: 0,
            attachments,
            forwarded,
            call: None,
            system: None,
            media: None,
            reads: None,
        }
    }

    fn attachment(kind: AttachmentKind, name: &str) -> Attachment {
        Attachment {
            kind,
            file_id: "f".into(),
            name: name.into(),
            size: 1,
            width: 0,
            height: 0,
        }
    }

    fn forwarded(text: &str, attachments: Vec<Attachment>) -> Forwarded {
        Forwarded {
            author: UserId("boris".into()),
            author_name: "Boris".into(),
            ts: 0,
            chat_id: None,
            text: text.into(),
            attachments,
            quote: None,
            media: None,
            reply: false,
        }
    }

    #[test]
    fn delete_preview_describes_a_message_without_text() {
        let s = mostik_core::mock::demo(1_794_300_000_000_000);
        let preview = |m: Message| delete_preview(&s, &m);
        let pdf = || vec![attachment(AttachmentKind::File, "report.pdf")];
        assert_eq!(preview(message("**Hi**", pdf(), vec![])).as_deref(), Some("Hi"));
        assert_eq!(preview(message("", pdf(), vec![])).as_deref(), Some("📄 report.pdf"));
        let image = vec![attachment(AttachmentKind::Image, "image.png")];
        assert_eq!(preview(message(" ", image, vec![])).as_deref(), Some("🖼 image.png"));
        let fwd = vec![forwarded("Plan", vec![])];
        assert_eq!(preview(message("", vec![], fwd)).as_deref(), Some("↪ Plan"));
        let fwd = vec![forwarded("", pdf())];
        assert_eq!(preview(message("", vec![], fwd)).as_deref(), Some("↪ 📄 report.pdf"));
        assert_eq!(preview(message("", vec![], vec![forwarded("", vec![])])), None);
        assert_eq!(preview(message("", vec![], vec![])), None);
        // A forwarded voice message or sticker has no text: it says what it is.
        let sticker = Forwarded {
            media: Some(Media::Sticker(Sticker {
                id: "a".into(),
                set_id: "b".into(),
            })),
            ..forwarded("", vec![])
        };
        let line = format!("↪ {}", tr().sticker);
        assert_eq!(preview(message("", vec![], vec![sticker.clone()])), Some(line.clone()));
        assert_eq!(message_preview(&s, &message("", vec![], vec![sticker])), line);
    }
}

/// The messages with files on their way in the open chat, or in its open thread for the thread
/// pane, at the end of the feed: the sender, the text and each file with how far its upload has
/// got, as the official client shows them. A click on the circle of a file cancels it; a message
/// that failed says so, with "Retry" and "Remove".
#[component]
pub(super) fn Outbox(#[prop(optional)] in_thread: bool) -> impl IntoView {
    let c = ctx();
    let here = Memo::new(move |_| {
        let chat = c.current.get();
        let thread = if in_thread { c.thread.get() } else { None };
        if in_thread && thread.is_none() {
            return vec![];
        }
        c.outbox.with(|o| {
            o.iter()
                .filter(|m| m.chat == chat && m.thread == thread)
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    move || {
        here.get()
            .into_iter()
            .map(|m| view! { <OutgoingRow out=m/> })
            .collect_view()
    }
}

/// Why a sending failed, in the interface's language where the backend's words are known.
fn failure_reason(why: &str) -> String {
    // Telemost refused the file itself (empty, or of a kind it does not take): a retry cannot help.
    if let Some(name) = why
        .strip_suffix(mostik_ipc::cmd::FILE_REFUSED)
        .and_then(|n| n.strip_suffix(": "))
    {
        return tr().file_refused(name);
    }
    let known = [
        (mostik_ipc::cmd::UPLOAD_STALLED, tr().upload_stalled),
        (mostik_ipc::cmd::DISK_FULL, tr().disk_full),
        (mostik_ipc::cmd::DISK_FILE_TOO_LARGE, tr().disk_file_too_large),
    ];
    known
        .into_iter()
        .find_map(|(theirs, ours)| {
            let name = why.strip_suffix(theirs)?.strip_suffix(": ")?;
            Some(format!("{name}: {ours}"))
        })
        .unwrap_or_else(|| ops::in_own_words(why))
}

/// One [`ops::Outgoing`] message.
#[component]
fn OutgoingRow(out: ops::Outgoing) -> impl IntoView {
    let c = ctx();
    let me = c.snap.with_untracked(|s| s.user(&s.me).cloned());
    let name = me.as_ref().map(|u| u.display_name.clone()).unwrap_or_default();
    let face = me.map(|u| view! { <Avatar user=u size=36/> });
    let failed = out.failed.clone();
    let status = match &failed {
        Some(_) => view! { <span class="out-status failed">{tr().not_sent}</span> }.into_any(),
        None => view! { <span class="out-status">{tr().sending}</span> }.into_any(),
    };
    let text = (!out.text.trim().is_empty()).then(|| {
        let who = c.snap.with_untracked(|s| s.mention_names());
        let me = c.snap.with_untracked(|s| s.me.clone());
        let body = who.with(|who| render_text(&out.text, who, Me { id: &me.0, name: &name }, false));
        view! { <div class="msg-text">{body}</div> }
    });
    let id = out.id.clone();
    let files = out
        .files
        .iter()
        .map(|f| {
            let (id, path) = (id.clone(), f.file.path.clone());
            let total = f.file.size.max(1);
            let share = (f.sent.min(total) as f64 / total as f64).clamp(0.0, 1.0);
            // A circle of radius 9: its length is 2πr.
            let length = 2.0 * std::f64::consts::PI * 9.0;
            let dash = format!("{:.1} {length:.1}", share * length);
            let can_cancel = !f.cancelled && !f.done() && failed.is_none();
            // A message that was not sent can go without any of its files: one uploaded whole may
            // still be the one Telemost refused.
            let can_remove = !f.cancelled && failed.is_some();
            let (remove_id, remove_path) = (id.clone(), path.clone());
            let ring = view! {
                <svg class="out-ring" viewBox="0 0 24 24" width="24" height="24">
                    <circle class="out-ring-track" cx="12" cy="12" r="9"/>
                    <circle class="out-ring-fill" cx="12" cy="12" r="9" stroke-dasharray=dash transform="rotate(-90 12 12)"/>
                    {can_cancel.then(|| view! { <path class="out-ring-x" d="M9 9l6 6M15 9l-6 6"/> })}
                </svg>
            };
            view! {
                <div class="file-card out-file" class:cancelled=f.cancelled>
                    <span class="file-icon">{if f.file.image { "🖼" } else { "📄" }}</span>
                    <span class="file-meta">
                        <span class="file-name">{mostik_core::plain_file_name(&f.file.name)}</span>
                        <span class="file-size">{if f.cancelled { tr().upload_cancelled.to_owned() } else { human_size(f.file.size) }}</span>
                    </span>
                    {if can_cancel {
                        view! {
                            <button class="out-cancel" title=tr().cancel_upload on:click=move |_| ops::cancel_file(c, &id, &path)>{ring}</button>
                        }.into_any()
                    } else if can_remove {
                        view! {
                            <button class="icon-btn out-remove" title=tr().remove on:click=move |_| ops::remove_failed_file(c, &remove_id, &remove_path)>
                                <Icon icon=I::X size=16/>
                            </button>
                        }.into_any()
                    } else {
                        view! { <span class="out-cancel">{ring}</span> }.into_any()
                    }}
                </div>
            }
        })
        .collect_view();
    let attempts = out.attempts;
    let actions = failed.map(|why| {
        let (retry, remove) = (id.clone(), id.clone());
        let why = match attempts {
            0 | 1 => failure_reason(&why),
            n => format!("{} ({})", failure_reason(&why), tr().attempt(n)),
        };
        view! {
            <div class="out-failed">
                <span class="out-why">{why}</span>
                <button class="btn small" on:click=move |_| ops::retry_upload(c, &retry)>{tr().retry}</button>
                <button class="btn small" on:click=move |_| ops::drop_upload(c, &remove)>{tr().remove}</button>
            </div>
        }
    });
    view! {
        <div class="msg outgoing">
            <div class="msg-row">
                <div class="msg-gutter">{face}</div>
                <div class="msg-content">
                    <div class="msg-head">
                        <span class="msg-author">{name.clone()}</span>
                        {status}
                    </div>
                    {text}
                    <div class="attachments">{files}</div>
                    {actions}
                </div>
            </div>
        </div>
    }
}
