//! Message text as views: paragraphs and code blocks, inline markup, links, mentions and custom
//! emoji.

use leptos::prelude::*;
use mostik_core::markup::{Inline, MENTION_ALL, Me, Mentionable, parse_inline};
use mostik_core::*;

use crate::i18n::tr;
use crate::ops;

use super::state::ctx;

/// Message text: paragraphs with inline markup and code blocks. The "edited" mark, if `edited`,
/// ends the last line, in a code block too, rather than taking a line of its own below the block.
pub(super) fn render_text(text: &str, who: &Mentionable, me: Me, edited: bool) -> AnyView {
    let blocks = mostik_core::markup::parse_blocks(text);
    let last = blocks.len().saturating_sub(1);
    let mark = move |i: usize| (edited && i == last).then(|| view! { <span class="edited">{tr().edited}</span> });
    if blocks.is_empty() {
        return mark(0).into_any();
    }
    blocks
        .into_iter()
        .enumerate()
        .map(|(i, b)| match b {
            mostik_core::markup::Block::Code(code) => {
                view! { <pre class="code-block">{code}{mark(i)}</pre> }.into_any()
            }
            mostik_core::markup::Block::Text(t) => view! { {render_inline(&t, who, me)}{mark(i)} }.into_any(),
        })
        .collect_view()
        .into_any()
}

fn render_inline(text: &str, who: &Mentionable, me: Me) -> AnyView {
    parse_inline(text, who)
        .into_iter()
        .map(|p| match p {
            Inline::Text(t) => custom_text(t),
            Inline::Bold(t) => view! { <b>{t}</b> }.into_any(),
            Inline::Italic(t) => view! { <i>{t}</i> }.into_any(),
            Inline::Strike(t) => view! { <s>{t}</s> }.into_any(),
            Inline::Underline(t) => view! { <u>{t}</u> }.into_any(),
            Inline::Code(t) => view! { <code>{t}</code> }.into_any(),
            Inline::Link { text, url } => {
                let (target, middle) = (url.clone(), url.clone());
                // A middle click is no `click`: left to the webview, WebKitGTK opens the link
                // right in the app window. It opens in the default browser, like a click. Its
                // release would also paste the X11 selection into the focused message field.
                view! {
                    <a class="msg-link" href=url.clone() title=url on:click=move |e| {
                        e.prevent_default();
                        ops::open_url(ctx(), target.clone());
                    } on:auxclick=move |e| {
                        e.prevent_default();
                        if e.button() == 1 {
                            ops::open_url(ctx(), middle.clone());
                        }
                    } on:mouseup=|e| {
                        if e.button() == 1 {
                            e.prevent_default();
                        }
                    }>{text}</a>
                }
                .into_any()
            }
            Inline::Mention(n) => {
                let cls = if n == me.name { "mention me" } else { "mention" };
                view! { <span class=cls>"@"{n}</span> }.into_any()
            }
            Inline::UserMention { id, name } => {
                let cls = if id == me.id {
                    "mention me person"
                } else {
                    "mention person"
                };
                let name = name.unwrap_or_else(|| tr().unknown_user.to_owned());
                let profile = UserId(id);
                view! {
                    <span class=cls on:click=move |e| {
                        e.stop_propagation();
                        ops::open_profile(ctx(), profile.clone());
                    }>"@"{name}</span>
                }
                .into_any()
            }
            // The user is in every public chat that can be mentioned, and in the chat for `@all`.
            Inline::ChannelMention(n) => view! { <span class="mention me group">"@"{n}</span> }.into_any(),
            Inline::MentionAll => view! { <span class="mention me group">"@"{MENTION_ALL}</span> }.into_any(),
        })
        .collect_view()
        .into_any()
}

/// Plain text with the custom emoji of its `:name:` codes as images.
fn custom_text(t: String) -> AnyView {
    let set = crate::custom::current(ctx());
    let Some(set) = set.filter(|s| !s.emoji.is_empty() && t.contains(':')) else {
        return view! { <span>{t}</span> }.into_any();
    };
    mostik_core::custom_emoji::split(&t, |n| set.by_name(n).is_some())
        .into_iter()
        .map(|p| match p {
            mostik_core::custom_emoji::Piece::Text(x) => view! { <span>{x.to_owned()}</span> }.into_any(),
            mostik_core::custom_emoji::Piece::Emoji(n) => set
                .by_name(n)
                .map(|e| crate::custom::image(e, "custom-emoji").into_any())
                .unwrap_or_else(|| view! { <span>{format!(":{n}:")}</span> }.into_any()),
        })
        .collect_view()
        .into_any()
}
