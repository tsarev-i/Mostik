//! Custom emoji in the UI (see `mostik_core::custom_emoji`): the set of the organization, lookups
//! by name and reaction type, and their images.

use leptos::prelude::*;
use mostik_core::ChatId;
use mostik_core::custom_emoji::{self as ce, CustomEmoji, EmojiSet};

use crate::app::{Ctx, current_org};
use crate::bridge;

/// The set of the organization the user is in now.
pub fn current(c: Ctx) -> Option<EmojiSet> {
    let org = current_org(c).map(|o| o.id);
    c.custom.with(|sets| ce::set_for(sets, &[], org).cloned())
}

/// The set that applies in `chat`: its organization's, else the current one's.
pub fn for_chat(c: Ctx, chat: &ChatId) -> Option<EmojiSet> {
    let orgs = c
        .snap
        .with_untracked(|s| s.chat(chat).map(|ch| ch.organizations.clone()).unwrap_or_default());
    let org = current_org(c).map(|o| o.id);
    c.custom.with(|sets| ce::set_for(sets, &orgs, org).cloned())
}

/// The custom emoji of reaction `kind` in `chat`.
pub fn by_kind(c: Ctx, chat: &ChatId, kind: u64) -> Option<CustomEmoji> {
    if !ce::is_custom_reaction(kind) {
        return None;
    }
    for_chat(c, chat).and_then(|s| s.by_kind(kind).cloned())
}

/// The image of an emoji; `class` sets its size.
pub fn image(e: &CustomEmoji, class: &'static str) -> impl IntoView + use<> {
    let code = ce::caption(&e.name);
    view! { <img class=class src=bridge::emoji_url(&e.file_id) alt=code.clone() title=code draggable="false"/> }
}

/// The image of a custom emoji inside a control that has its own tooltip (a reaction says who
/// reacted): no `title` of its own, which would cover the control's.
pub fn image_untitled(e: &CustomEmoji, class: &'static str) -> impl IntoView + use<> {
    view! { <img class=class src=bridge::emoji_url(&e.file_id) alt=ce::caption(&e.name) draggable="false"/> }
}

/// Emoji names starting with or containing `query`, for the `:` suggestions in the composer.
pub fn suggest(c: Ctx, query: &str, limit: usize) -> Vec<CustomEmoji> {
    let q = query.to_lowercase();
    let Some(set) = current(c) else { return vec![] };
    let (mut starts, contains): (Vec<CustomEmoji>, Vec<CustomEmoji>) = set
        .emoji
        .into_iter()
        .filter(|e| e.name.contains(&q))
        .partition(|e| e.name.starts_with(&q));
    starts.extend(contains);
    starts.truncate(limit);
    starts
}

/// Image URLs of the current organization's emoji by name, for the message field.
pub fn lookup(c: Ctx) -> impl Fn(&str) -> Option<String> {
    let set = current(c);
    move |name: &str| {
        set.as_ref()
            .and_then(|s| s.by_name(name))
            .map(|e| bridge::emoji_url(&e.file_id))
    }
}
