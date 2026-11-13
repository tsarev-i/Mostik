//! The message fields: the in-place editor of a message and the composer of a chat or thread.

use std::time::Duration;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::markup::MENTION_ALL;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::{bridge, ops};

use super::avatar::Avatar;
use super::feed::typing_line;
use super::field::{
    Suggestion, emoji_query, insert_text, mention_query, mention_suggestions, place_caret, selection, utf16_len,
};
use super::message::{ReplyBar, human_size};
use super::state::{ctx, enter_sends};

/// Text typed in the editor of a message and where the caret was.
pub struct EditDraft {
    pub(super) key: (ChatId, Timestamp),
    text: String,
    caret: Option<u32>,
}

/// In-place message editing, as in Slack: a text field, "Cancel" and "Save".
/// Enter saves (or Ctrl+Enter, depending on the preference), Esc cancels.
#[component]
pub(super) fn InlineEditor(msg: Message) -> impl IntoView {
    let c = ctx();
    // Mentions are edited by name and sent back as ids.
    let (shown, named) = c.snap.with_untracked(|s| {
        s.mention_names()
            .with(|who| mostik_core::markup::ids_to_names(&msg.text, who))
    });
    c.picked
        .update(|p| p.extend(named.into_iter().map(|(name, id)| (name, UserId(id)))));
    let original = shown.trim().to_owned();
    // The line may have been built again while the user was typing: go on with their draft.
    let key = (msg.chat_id.clone(), msg.ts);
    let draft = c
        .edit_draft
        .with_value(|d| d.as_ref().filter(|d| d.key == key).map(|d| (d.text.clone(), d.caret)));
    let (initial, caret) = draft.unwrap_or((shown, None));
    let caret = caret.unwrap_or(utf16_len(&initial));
    let text = RwSignal::new(initial);
    let busy = RwSignal::new(false);
    let area = NodeRef::<html::Div>::new();
    Effect::new(move |first: Option<()>| {
        let text = text.get();
        if first.is_none() {
            return;
        }
        let caret = area.get_untracked().and_then(|el| selection(&el).1);
        c.edit_draft.set_value(Some(EditDraft {
            key: key.clone(),
            text,
            caret,
        }));
    });
    let initial = text.get_untracked();
    Effect::new(move |_| {
        if let Some(el) = area.get() {
            // Only the field appearing fills it: the emoji lookup reads the snapshot, and its
            // changes (someone came online) would put the original text back over the typed one.
            untrack(|| place_caret(&el, &initial, caret));
            // The editor is taller than the message: the last message's editor would go under
            // the composer. Once laid out, scroll it fully into view.
            request_animation_frame(move || {
                if let Some(el) = area.try_get_untracked().flatten() {
                    reveal_editor(&el);
                }
            });
        }
    });
    let cancel = move || c.editing.set(None);
    let save = {
        let msg = msg.clone();
        move || {
            let t = text.get_untracked().trim().to_owned();
            if busy.get_untracked() {
                return;
            }
            if t.is_empty() {
                // As in Slack: empty text offers to delete the message.
                c.editing.set(None);
                return c.confirm_delete.set(Some(msg.clone()));
            }
            if t == original {
                return c.editing.set(None);
            }
            busy.set(true);
            let m = msg.clone();
            spawn_local(async move {
                match ops::edit(c, m, t).await {
                    Ok(()) => c.editing.set(None),
                    Err(e) => ops::failed(c, tr().err_not_sent, e),
                }
                // Saved, the editor closes and its signals go.
                busy.try_set(false);
            });
        }
    };
    let save_key = save.clone();
    let field = crate::compose::Field::new(area, move |t| text.set(t));
    view! {
        <div class="inline-edit" on:click=|e| e.stop_propagation()>
            <div class="inline-edit-box">
                <div class="editor" node_ref=area contenteditable="plaintext-only" role="textbox" aria-multiline="true"
                    on:input=move |_| {
                        if let Some(el) = area.get_untracked() {
                            text.set(crate::editor::tidy(&el, &crate::custom::lookup(c)));
                            reveal_editor(&el);
                        }
                    }
                    on:beforeinput=move |e: ev::InputEvent| {
                        if let Some(el) = area.get_untracked() {
                            crate::editor::before_input(&el, &e, &crate::custom::lookup(c));
                        }
                    }
                    on:paste=move |e: ev::ClipboardEvent| { field.paste_link(&e); }
                    on:keydown=move |e: ev::KeyboardEvent| {
                        if e.is_composing() || field.hotkey(&e) { return; }
                        match e.key().as_str() {
                            "Escape" => { e.prevent_default(); e.stop_propagation(); cancel(); }
                            "Enter" if (enter_sends() && !e.shift_key() && !e.ctrl_key()) || (!enter_sends() && e.ctrl_key()) => {
                                e.prevent_default();
                                save_key();
                            }
                            "Enter" => {
                                e.prevent_default();
                                if let Some(el) = area.get_untracked() {
                                    text.set(insert_text(&el, "\n", 0));
                                }
                            }
                            _ => {}
                        }
                    }></div>
                <crate::compose::FormatBar field emoji=true/>
            </div>
            <div class="inline-edit-actions">
                <button class="btn small" on:click=move |_| cancel()>{tr().cancel}</button>
                <button class="btn small primary" disabled=move || busy.get() on:click=move |_| save()>
                    <Icon icon=I::Enter size=14/>{tr().save_changes}
                </button>
            </div>
        </div>
    }
}

/// Scrolls the message list just enough for the whole in-place editor (text and buttons) to show,
/// with the gap from `scroll-margin` in the CSS.
fn reveal_editor(area: &web_sys::HtmlElement) {
    if let Ok(Some(editor)) = area.closest(".inline-edit") {
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
        editor.scroll_into_view_with_scroll_into_view_options(&opts);
    }
}

#[component]
pub(super) fn Composer(#[prop(optional)] thread: bool) -> impl IntoView {
    let c = ctx();
    let (draft, set_draft) = signal(String::new());
    let sending = RwSignal::new(false);
    let area = NodeRef::<html::Div>::new();
    let parent = move || if thread { c.thread.get_untracked() } else { None };
    let suggestions: RwSignal<Vec<Suggestion>> = RwSignal::new(vec![]);
    let suggest_sel = RwSignal::new(0usize);

    // The field shows the draft: a restored draft, a sent message clearing it, formatting. Typing
    // already shows, so the field is filled again only when its text differs, or when the emoji set
    // arrives and turns codes into images. The caret stays where it was.
    Effect::new(move |_| {
        let text = draft.get();
        let lookup = crate::custom::lookup(c);
        let Some(el) = area.get() else { return };
        let shown = crate::editor::value(&el);
        if shown == text {
            crate::editor::tidy(&el, &lookup);
            return;
        }
        let focused = document().active_element().is_some_and(|a| a == **el);
        crate::editor::render(&el, &text, &lookup);
        if focused {
            crate::editor::set_caret(&el, utf16_len(&text));
        }
    });

    // Chat draft: fill it in on opening, save it while typing (main composer only). The draft keeps
    // only the text: a reply started in another chat ends when the chat changes.
    Effect::new(move |_| {
        let id = c.current.get();
        if !thread {
            set_draft.set(c.drafts.with_untracked(|d| d.get(&id).cloned().unwrap_or_default()));
            if c.replying
                .with_untracked(|r| r.as_ref().is_some_and(|r| r.msg.chat_id != id))
            {
                c.replying.set(None);
            }
        }
    });

    // The reply this field sends: to a message of its chat, in its thread for the thread field.
    let reply = Memo::new(move |_| {
        let (chat, place) = (c.current.get(), if thread { c.thread.get() } else { None });
        if thread && place.is_none() {
            return None;
        }
        c.replying.with(|r| {
            r.clone()
                .filter(|r| r.msg.chat_id == chat && r.msg.thread_parent == place)
        })
    });
    // Starting a reply puts the caret at the end of this field.
    Effect::new(move |prev: Option<Option<(ChatId, Timestamp)>>| {
        let now = reply.with(|r| r.as_ref().map(|r| (r.msg.chat_id.clone(), r.msg.ts)));
        if now.is_some()
            && prev.flatten() != now
            && let Some(el) = area.get_untracked()
        {
            let _ = el.focus();
            crate::editor::set_caret(&el, utf16_len(&crate::editor::value(&el)));
        }
        now
    });
    let remember = move |text: &str| {
        if thread {
            return;
        }
        let id = c.current.get_untracked();
        c.drafts.update(|d| {
            if text.trim().is_empty() {
                d.remove(&id);
            } else {
                d.insert(id, text.to_owned());
            }
        });
    };

    let placeholder = move || {
        if thread {
            return tr().reply_placeholder.to_owned();
        }
        let id = c.current.get();
        c.snap.with(|s| {
            s.chat(&id)
                .map(|ch| match ch.kind {
                    ChatKind::Direct { .. } => tr().message_to_person(&chat_title(s, ch)),
                    ChatKind::Notes => tr().note_placeholder.to_owned(),
                    _ => tr().message_to_channel(split_chat_icon(&ch.name).1),
                })
                .unwrap_or_default()
        })
    };
    let submit = move || {
        let text = draft.get_untracked().trim().to_owned();
        let with_files = !thread && !ops::attached(c, &c.current.get_untracked()).is_empty();
        if (text.is_empty() && !with_files) || sending.get_untracked() {
            return;
        }
        let chat = c.current.get_untracked();
        let chat_for_draft = chat.clone();
        let thread_parent = parent();
        let sent_text = text.clone();
        let replying = reply.get_untracked();
        sending.set(true);
        spawn_local(async move {
            // A reply is a text message: attached files follow it without a caption. Files alone
            // keep the reply bar for the text that comes next.
            let replied = replying.is_some() && !text.is_empty();
            let result = match (replying.clone(), with_files) {
                (Some(r), true) if !text.is_empty() => {
                    match ops::send(c, chat.clone(), text, thread_parent, Some(r)).await {
                        Ok(()) => ops::send_files(c, chat, String::new(), thread_parent).await,
                        Err(e) => Err(e),
                    }
                }
                (_, true) => ops::send_files(c, chat, text, thread_parent).await,
                (r, false) => ops::send(c, chat, text, thread_parent, r).await,
            };
            // The composer may be gone by now (the user left the chat view while files uploaded).
            sending.try_set(false);
            if result.is_ok() && replied && c.replying.get_untracked() == replying {
                c.replying.set(None);
            }
            match result {
                Ok(()) => {
                    // Clear only what was sent: while the message was on its way the user may have
                    // switched to another chat or thread, or kept typing.
                    let unchanged = |d: &str| d.trim() == sent_text;
                    if !thread {
                        c.drafts.update(|d| {
                            if d.get(&chat_for_draft).is_some_and(|t| unchanged(t)) {
                                d.remove(&chat_for_draft);
                            }
                        });
                    }
                    let same_place = c.current.get_untracked() == chat_for_draft && parent() == thread_parent;
                    if same_place && draft.try_get_untracked().is_some_and(|d| unchanged(&d)) {
                        set_draft.set(String::new());
                    }
                }
                Err(e) => ops::failed(c, tr().err_not_sent, e),
            }
        });
    };
    // Picking a suggestion replaces the typed `:code` or `@name`.
    let accept_suggestion = move |i: usize| {
        let Some(pick) = suggestions.with_untracked(|s| s.get(i).cloned()) else {
            return;
        };
        if let Some(el) = area.get_untracked() {
            let text = match pick {
                Suggestion::Emoji { glyph, code } => {
                    crate::emoji::remember(crate::emoji::Source::Text, glyph);
                    insert_text(
                        &el,
                        glyph,
                        code.len().min(emoji_query(&el).map_or(0, |q| q.chars().count())) + 1,
                    )
                }
                Suggestion::Custom(e) => {
                    let code = mostik_core::custom_emoji::caption(&e.name);
                    crate::emoji::remember(crate::emoji::Source::Text, &code);
                    insert_text(&el, &code, emoji_query(&el).map_or(0, |q| q.chars().count()) + 1)
                }
                Suggestion::Person(_) | Suggestion::Channel(_) | Suggestion::All => {
                    let name = match pick {
                        Suggestion::Person(u) => {
                            c.picked.update(|p| {
                                p.insert(u.display_name.clone(), u.id);
                            });
                            u.display_name
                        }
                        Suggestion::Channel(n) => n,
                        _ => MENTION_ALL.to_owned(),
                    };
                    let typed = mention_query(&el).map_or(0, |q| q.chars().count() + 1);
                    insert_text(&el, &format!("@{name} "), typed)
                }
            };
            remember(&text);
            set_draft.set(text);
        }
        suggestions.set(vec![]);
    };
    let field = crate::compose::Field::new(area, move |text| {
        remember(&text);
        set_draft.set(text);
    });
    if !thread {
        crate::compose::attach_dropped(c);
    }
    let on_key = move |ev: web_sys::KeyboardEvent| {
        if ev.is_composing() || field.hotkey(&ev) {
            return;
        }
        let n = suggestions.with_untracked(Vec::len);
        if n > 0 {
            match ev.key().as_str() {
                "ArrowDown" => {
                    ev.prevent_default();
                    return suggest_sel.update(|i| *i = (*i + 1) % n);
                }
                "ArrowUp" => {
                    ev.prevent_default();
                    return suggest_sel.update(|i| *i = (*i + n - 1) % n);
                }
                "Enter" | "Tab" => {
                    ev.prevent_default();
                    return accept_suggestion(suggest_sel.get_untracked());
                }
                "Escape" => {
                    // Closes only the suggestions, not the reply bar.
                    ev.prevent_default();
                    ev.stop_propagation();
                    return suggestions.set(vec![]);
                }
                _ => {}
            }
        }
        match ev.key().as_str() {
            "Enter" if (enter_sends() && !ev.shift_key() && !ev.ctrl_key()) || (!enter_sends() && ev.ctrl_key()) => {
                ev.prevent_default();
                submit();
            }
            // A line break of our own: the browser would put the new line in a block.
            "Enter" => {
                ev.prevent_default();
                if let Some(el) = area.get_untracked() {
                    let text = insert_text(&el, "\n", 0);
                    remember(&text);
                    set_draft.set(text);
                }
            }
            // ↑ in an empty field edits your last message, as in Slack.
            "ArrowUp" if draft.with_untracked(String::is_empty) => {
                let chat = c.current.get_untracked();
                let tp = parent();
                let last = c.snap.with_untracked(|s| {
                    s.messages
                        .iter()
                        .filter(|m| m.chat_id == chat && m.author == s.me && m.thread_parent == tp && !m.is_note())
                        .max_by_key(|m| m.ts)
                        .cloned()
                });
                if let Some(m) = last {
                    ev.prevent_default();
                    c.editing.set(Some(m));
                }
            }
            _ => {}
        }
    };
    let typing_text = move || if thread { None } else { typing_line(c, &c.current.get()) };
    // The files attached in the open chat.
    let files = move || {
        let chat = c.current.get();
        c.pending_files.with(|all| all.get(&chat).cloned().unwrap_or_default())
    };
    view! {
        <div class=if thread { "composer-wrap in-thread" } else { "composer-wrap" }>
            <div class="composer">
                {move || reply.get().map(|r| view! { <ReplyBar target=r/> })}
                {(!thread).then(|| view! {
                    <Show when=move || !files().is_empty()>
                        <div class="pending-files">
                            {move || files().into_iter().map(|f| {
                                let path = f.path.clone();
                                view! {
                                    <span class="pending-file">
                                        <span class="file-icon small">{if f.image { "🖼" } else { "📄" }}</span>
                                        <span class="file-meta">
                                            <span class="file-name">{f.name.clone()}</span>
                                            <span class="file-size">{human_size(f.size)}</span>
                                        </span>
                                        <button class="pf-remove" title=tr().remove_file
                                            on:click=move |_| ops::detach_file(c, &c.current.get_untracked(), &path)>
                                            <Icon icon=I::X size=12/>
                                        </button>
                                    </span>
                                }
                            }).collect_view()}
                        </div>
                    </Show>
                })}
                <crate::compose::FormatBar field/>
                // A placeholder of its own: WebKit does not always restyle `attr()` in `::before` when
                // the chat, and so the attribute, changes.
                <div class="editor-box">
                    <div class="editor-placeholder" hidden=move || !draft.with(String::is_empty)>{placeholder}</div>
                    <div
                        class="editor"
                        node_ref=area
                        contenteditable="plaintext-only"
                        role="textbox"
                        aria-multiline="true"
                        aria-placeholder=placeholder
                        on:input=move |_| {
                            let Some(text) = area.get_untracked().map(|el| crate::editor::tidy(&el, &crate::custom::lookup(c))) else { return };
                            remember(&text);
                            set_draft.set(text);
                            ops::typing(c, c.current.get_untracked());
                            let found = area.get_untracked().map(|el| {
                                if let Some(q) = emoji_query(&el) {
                                    // Custom emoji first, as in Slack, leaving room for the standard ones.
                                    let standard = crate::emoji::suggest(&q, 8);
                                    let mut found: Vec<Suggestion> = crate::custom::suggest(c, &q, 8).into_iter().map(Suggestion::Custom).collect();
                                    found.truncate(8usize.saturating_sub(standard.len()).max(4));
                                    found.extend(standard.into_iter().map(|(glyph, code)| Suggestion::Emoji { glyph, code }));
                                    found.truncate(8);
                                    found
                                } else if let Some(q) = mention_query(&el) {
                                    let chat = c.current.get_untracked();
                                    c.snap.with_untracked(|s| mention_suggestions(s, &chat, &q, 8))
                                } else {
                                    vec![]
                                }
                            }).unwrap_or_default();
                            suggest_sel.set(0);
                            suggestions.set(found);
                        }
                        on:beforeinput=move |e: ev::InputEvent| {
                            if let Some(el) = area.get_untracked() {
                                crate::editor::before_input(&el, &e, &crate::custom::lookup(c));
                            }
                        }
                        on:keydown=on_key
                        on:paste=move |e: ev::ClipboardEvent| {
                            if thread || !crate::compose::attach_pasted(c, &e) {
                                field.paste_link(&e);
                            }
                        }
                        on:blur=move |_| set_timeout(move || { suggestions.try_set(vec![]); }, Duration::from_millis(150))
                    ></div>
                </div>
                <Show when=move || !suggestions.with(Vec::is_empty)>
                    <div class="emoji-suggest">
                        {move || suggestions.get().into_iter().enumerate().map(|(i, pick)| {
                            let row = match pick {
                                Suggestion::Emoji { glyph, code } => view! {
                                    <span class="glyph">{glyph}</span><span>{format!(":{code}:")}</span>
                                }.into_any(),
                                Suggestion::Custom(e) => view! {
                                    <span class="glyph">{crate::custom::image(&e, "custom-emoji")}</span>
                                    <span>{mostik_core::custom_emoji::caption(&e.name)}</span>
                                }.into_any(),
                                Suggestion::Person(u) => {
                                    let name = u.display_name.clone();
                                    let title = u.title.clone().unwrap_or_default();
                                    view! {
                                        <span class="glyph"><Avatar user=u size=20/></span>
                                        <span class="name">{name}</span><span class="hint">{title}</span>
                                    }.into_any()
                                }
                                Suggestion::Channel(n) => view! {
                                    <span class="glyph"><Icon icon=I::Hash size=16/></span>
                                    <span class="name">{n}</span><span class="hint">{tr().mention_channel_hint}</span>
                                }.into_any(),
                                Suggestion::All => view! {
                                    <span class="glyph">"@"</span>
                                    <span class="name">{MENTION_ALL}</span><span class="hint">{tr().mention_all_hint}</span>
                                }.into_any(),
                            };
                            view! {
                                <button class:active=move || suggest_sel.get() == i
                                    on:mousedown=move |e| { e.prevent_default(); accept_suggestion(i); }>
                                    {row}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                </Show>
                <div class="composer-bottom">
                    {(!thread).then(|| view! {
                        <button class="cb round" title=tr().attach on:click=move |_| {
                            if !c.live {
                                return c.error.set(Some(tr().only_in_app_files.into()));
                            }
                            // The files go to the chat the dialog was opened in.
                            let chat = c.current.get_untracked();
                            spawn_local(async move {
                                match bridge::pick_files().await {
                                    Ok(files) => ops::attach_files(c, chat, files),
                                    Err(e) => c.error.set(Some(e)),
                                }
                            });
                        }><Icon icon=I::Plus size=16/></button>
                    })}
                    <crate::compose::EmojiButton field class="cb" size=18/>
                    {move || sending.get().then(|| view! { <span class="composer-hint">{tr().uploading}</span> })}
                    <span class="composer-hint">{if crate::app::enter_sends() { tr().hint_shift_enter } else { tr().hint_ctrl_enter }}</span>
                    <div class="spacer"></div>
                    <div class=move || if (draft.with(|d| d.trim().is_empty()) && (thread || files().is_empty())) || sending.get() { "send-group empty" } else { "send-group" }>
                        <button class="send" title=tr().send on:click=move |_| submit()><Icon icon=I::Send size=16/></button>
                    </div>
                </div>
            </div>
            <div class="typing">{typing_text}</div>
        </div>
    }
}
