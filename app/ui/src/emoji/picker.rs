//! Slack-style emoji picker: search, section tabs, "Frequently used" and a caption at the bottom.
//! One component for two cases: emoji in text (all of Unicode) and reactions (Telemost types only).
//! Both can add the organization's custom emoji as the last section, as in Slack.

use leptos::html;
use leptos::prelude::*;
use mostik_core::custom_emoji::{self as ce, EmojiSet};
use mostik_core::reactions;

use crate::i18n::tr;

use super::{Source, load_recent, remember};

/// Width and height of the picker, as in Slack.
const W: f64 = 352.0;
const H: f64 = 420.0;

/// What an emoji looks like: a character or the image of a custom emoji.
#[derive(Clone)]
enum Glyph {
    Char(&'static str),
    Image(ce::CustomEmoji),
}

/// How many pictures a string of emoji draws: a reaction like "4️⃣➕" is two, a ZWJ family or a flag one.
fn pictures(s: &str) -> usize {
    let mut count = 0;
    let (mut joined, mut flag_half) = (false, false);
    for c in s.chars() {
        let u = u32::from(c);
        if c == '\u{200D}' {
            joined = true;
            continue;
        }
        let modifier = matches!(u, 0xFE0E | 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F);
        if modifier || std::mem::take(&mut joined) {
            continue;
        }
        if (0x1F1E6..=0x1F1FF).contains(&u) {
            flag_half = !flag_half;
            if !flag_half {
                continue;
            }
        }
        count += 1;
    }
    count
}

impl Glyph {
    fn view(&self, class: &'static str) -> AnyView {
        match self {
            // Two pictures in one cell are drawn smaller so that the cell keeps its width.
            Glyph::Char(c) if pictures(c) > 1 => view! { <span class="ep-multi">{*c}</span> }.into_any(),
            Glyph::Char(c) => view! { <span>{*c}</span> }.into_any(),
            Glyph::Image(e) => crate::custom::image(e, class).into_any(),
        }
    }
}

/// Emoji in the picker: `key` is passed to `on_pick` (the character itself or the reaction type;
/// for a custom emoji its `:name:` or its reaction type).
#[derive(Clone)]
struct Item {
    key: String,
    glyph: Glyph,
    name: String,
    section: usize,
}

/// Section tab icon. The sections are the same as in Slack.
const TEXT_TABS: [&str; 9] = ["😀", "👋", "🐻", "🍔", "✈️", "⚽", "💡", "🔣", "🏳️"];

/// Index of the custom emoji section: after the standard ones.
fn custom_section(source: Source) -> usize {
    match source {
        Source::Text => TEXT_TABS.len(),
        Source::Reactions => 1,
    }
}

fn items(source: Source, custom: Option<&EmojiSet>) -> Vec<Item> {
    let mut items: Vec<Item> = match source {
        Source::Text => emojis::iter()
            .map(|e| Item {
                key: e.as_str().to_owned(),
                glyph: Glyph::Char(e.as_str()),
                name: e.shortcode().map_or_else(|| e.name().replace(' ', "_"), str::to_owned),
                section: e.group() as usize,
            })
            .collect(),
        Source::Reactions => reactions::all()
            .map(|(kind, name, glyph)| Item {
                key: kind.to_string(),
                glyph: Glyph::Char(glyph),
                name: name.to_lowercase(),
                section: 0,
            })
            .collect(),
    };
    for e in custom.into_iter().flat_map(|s| s.emoji.iter()) {
        items.push(Item {
            key: match source {
                Source::Text => ce::caption(&e.name),
                Source::Reactions => e.kind.to_string(),
            },
            glyph: Glyph::Image(e.clone()),
            name: e.name.clone(),
            section: custom_section(source),
        });
    }
    items
}

/// Picker position: above the button, or below it if it does not fit; never leaves the window.
fn place(anchor: (f64, f64)) -> String {
    let w = web_sys::window();
    let iw = w
        .as_ref()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(1280.0);
    let ih = w
        .as_ref()
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(800.0);
    let left = (anchor.0 - W).clamp(8.0, (iw - W - 8.0).max(8.0));
    let top = if anchor.1 - H - 6.0 >= 8.0 {
        anchor.1 - H - 6.0
    } else {
        (anchor.1 + 34.0).min(ih - H - 8.0).max(8.0)
    };
    format!("left:{left:.0}px;top:{top:.0}px")
}

#[component]
pub fn EmojiPicker(
    source: Source,
    anchor: (f64, f64),
    on_pick: Callback<String>,
    on_close: Callback<()>,
    /// Custom emoji to offer as well (not where only a character fits, like a section icon).
    #[prop(default = None)]
    custom: Option<EmojiSet>,
    /// "Add emoji" at the bottom, as in Slack: opens the "Add custom emoji" dialog.
    #[prop(optional)]
    can_add: bool,
) -> impl IntoView {
    let custom = custom.filter(|s| !s.emoji.is_empty());
    let all = StoredValue::new(items(source, custom.as_ref()));
    let recent = load_recent(source);
    let query = RwSignal::new(String::new());
    let hover: RwSignal<Option<(Glyph, String)>> = RwSignal::new(None);
    // An own custom emoji the user asked to delete (right click), waiting for confirmation.
    let deleting: RwSignal<Option<ce::CustomEmoji>> = RwSignal::new(None);
    let busy = RwSignal::new(false);
    let ctx = expect_context::<crate::app::Ctx>();
    let me = StoredValue::new(ctx.snap.with_untracked(|s| s.me.clone()));
    let service_chat = StoredValue::new(custom.as_ref().map(|s| s.chat_id.clone()));
    let body = NodeRef::<html::Div>::new();
    let search = NodeRef::<html::Input>::new();
    // Esc closes the picker wherever the focus is; the search box handles its own Esc.
    crate::app::scoped_window_listener(leptos::ev::keydown, move |e| {
        if e.key() == "Escape" {
            on_close.run(());
        }
    });
    Effect::new(move |_| {
        if let Some(el) = search.get() {
            let _ = el.focus();
        }
    });

    let mut sections: Vec<&'static str> = match source {
        Source::Text => tr().emoji_groups.to_vec(),
        Source::Reactions => vec![tr().all_reactions],
    };
    let mut tabs: Vec<Glyph> = match source {
        Source::Text => TEXT_TABS.iter().map(|t| Glyph::Char(t)).collect(),
        Source::Reactions => vec![Glyph::Char("😄")],
    };
    if let Some(first) = custom.as_ref().and_then(|s| s.emoji.first()) {
        sections.push(tr().custom_emoji);
        tabs.push(Glyph::Image(first.clone()));
    }
    let tab_titles = sections.clone();
    let recent_items: Vec<Item> = all.with_value(|all| {
        recent
            .iter()
            .filter_map(|k| all.iter().find(|i| &i.key == k).cloned())
            .collect()
    });

    let pick = move |key: String| {
        remember(source, &key);
        on_pick.run(key);
    };
    let cell = move |it: Item| {
        let (key, glyph, name) = (it.key.clone(), it.glyph.clone(), it.name.clone());
        let shown = glyph.view("ep-img");
        // Your own custom emoji is deleted with a right click, as there is no other place for it.
        let own = match &it.glyph {
            Glyph::Image(e) if me.with_value(|m| &e.owner == m) => Some(e.clone()),
            _ => None,
        };
        let title = own.is_some().then_some(tr().emoji_delete_hint);
        view! {
            <button class="ep-cell" title=title
                on:mouseenter=move |_| hover.set(Some((glyph.clone(), name.clone())))
                on:click=move |_| pick(key.clone())
                on:contextmenu=move |e| {
                    // The picker of a reaction sits in its message: no message menu from here.
                    e.prevent_default();
                    e.stop_propagation();
                    if let Some(own) = own.clone() {
                        deleting.set(Some(own));
                    }
                }>{shown}</button>
        }
    };
    let delete = move |e: ce::CustomEmoji| {
        let Some(chat) = service_chat.get_value() else { return };
        busy.set(true);
        leptos::task::spawn_local(async move {
            match crate::bridge::delete_custom_emoji(&chat.0, e.ts).await {
                Ok(sets) => {
                    ctx.custom.set(sets);
                    on_close.run(());
                }
                Err(err) => ctx.error.set(Some(err)),
            }
            busy.try_set(false);
        });
    };
    let jump = move |id: String| {
        query.set(String::new());
        if let Some(el) = body
            .get_untracked()
            .and_then(|b| b.query_selector(&format!("[data-sec='{id}']")).ok().flatten())
        {
            el.scroll_into_view();
        }
    };

    let grid = move || {
        let q = query.get().trim().to_lowercase();
        if !q.is_empty() {
            let found: Vec<Item> =
                all.with_value(|all| all.iter().filter(|i| i.name.contains(&q)).take(300).cloned().collect());
            return if found.is_empty() {
                view! { <div class="ep-empty">{tr().no_emoji_found}</div> }.into_any()
            } else {
                view! { <div class="ep-grid">{found.into_iter().map(cell).collect_view()}</div> }.into_any()
            };
        }
        let rec = recent_items.clone();
        view! {
            {(!rec.is_empty()).then(|| view! {
                <div class="ep-sec" data-sec="recent">{tr().frequently_used}</div>
                <div class="ep-grid">{rec.into_iter().map(cell).collect_view()}</div>
            })}
            {sections.iter().enumerate().map(|(i, title)| {
                let list: Vec<Item> = all.with_value(|all| all.iter().filter(|it| it.section == i).cloned().collect());
                view! {
                    <div class="ep-sec" data-sec=i.to_string()>{*title}</div>
                    <div class="ep-grid">{list.into_iter().map(cell).collect_view()}</div>
                }
            }).collect_view()}
        }
        .into_any()
    };

    view! {
        <div class="ep-backdrop" on:click=move |_| on_close.run(()) on:contextmenu=move |e| { e.prevent_default(); on_close.run(()); }></div>
        <div class="emoji-picker" style=place(anchor) on:click=|e| e.stop_propagation()
            on:contextmenu=|e| { e.prevent_default(); e.stop_propagation(); }>
            <div class="ep-search">
                <input node_ref=search class="text-input" placeholder=match source { Source::Text => tr().search_emoji, Source::Reactions => tr().search_reactions }
                    prop:value=move || query.get()
                    on:input=move |e| query.set(event_target_value(&e))
                    on:keydown=move |e| if e.key() == "Escape" { e.stop_propagation(); on_close.run(()); }/>
            </div>
            <div class="ep-tabs">
                <button title=tr().frequently_used on:click=move |_| jump("recent".into())>"🕘"</button>
                {tabs.into_iter().enumerate().map(|(i, t)| {
                    let title = tab_titles.get(i).copied().unwrap_or_default();
                    view! { <button title=title on:click=move |_| jump(i.to_string())>{t.view("ep-tab-img")}</button> }
                }).collect_view()}
            </div>
            <div class="ep-body" node_ref=body on:mouseleave=move |_| hover.set(None)>{grid}</div>
            <div class="ep-foot">
                {move || match (deleting.get(), hover.get()) {
                    (Some(e), _) => {
                        let gone = e.clone();
                        view! {
                            <span class="ep-big">{Glyph::Image(e.clone()).view("ep-big-img")}</span>
                            <span class="ep-name">{tr().emoji_delete_ask(&e.name)}</span>
                            <button class="btn small" on:click=move |_| deleting.set(None)>{tr().cancel}</button>
                            <button class="btn small danger" disabled=move || busy.get() on:click=move |_| delete(gone.clone())>{tr().delete}</button>
                        }.into_any()
                    }
                    (None, Some((glyph, name))) => view! { <span class="ep-big">{glyph.view("ep-big-img")}</span><span class="ep-name">{format!(":{name}:")}</span> }.into_any(),
                    (None, None) => view! { <span class="ep-big">"🙂"</span><span class="ep-name muted">{tr().pick_emoji}</span> }.into_any(),
                }}
                {can_add.then(|| view! {
                    <button class="btn small ep-add" on:click=move |_| {
                        on_close.run(());
                        ctx.adding_emoji.set(true);
                    }>{tr().add_emoji}</button>
                })}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_match_labels() {
        assert_eq!(emojis::Group::iter().count(), TEXT_TABS.len());
        assert_eq!(
            crate::i18n::for_lang(crate::i18n::Lang::En).emoji_groups.len(),
            TEXT_TABS.len()
        );
    }

    #[test]
    fn counts_drawn_pictures() {
        assert_eq!(pictures("👍"), 1);
        assert_eq!(pictures("❤️"), 1);
        assert_eq!(pictures("👍🏽"), 1);
        assert_eq!(pictures("👨‍👩‍👧"), 1);
        assert_eq!(pictures("🇷🇺"), 1);
        assert_eq!(pictures("4️⃣"), 1);
        assert_eq!(pictures("4️⃣➕"), 2);
    }
}
