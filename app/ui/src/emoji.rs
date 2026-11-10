//! Slack-style emoji picker: search, section tabs, "Frequently used" and a caption at the bottom.
//! One component for two cases: emoji in text (all of Unicode) and reactions (Telemost types only).

use leptos::html;
use leptos::prelude::*;
use mostik_core::reactions;

use crate::i18n::tr;

/// Width and height of the picker, as in Slack.
const W: f64 = 352.0;
const H: f64 = 420.0;
/// How many emoji to remember in "Frequently used".
const RECENT_MAX: usize = 27;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Any Unicode emoji: inserted into the message text.
    Text,
    /// Telemost reactions: the server accepts only types `100101…100157`.
    Reactions,
}

/// Emoji in the picker: `key` is passed to `on_pick` (the character itself or the reaction type).
#[derive(Clone)]
struct Item {
    key: String,
    glyph: &'static str,
    name: String,
    section: usize,
}

/// Section tab icon. The sections are the same as in Slack.
const TEXT_TABS: [&str; 9] = ["😀", "👋", "🐻", "🍔", "✈️", "⚽", "💡", "🔣", "🏳️"];

fn items(source: Source) -> Vec<Item> {
    match source {
        Source::Text => emojis::iter()
            .map(|e| Item {
                key: e.as_str().to_owned(),
                glyph: e.as_str(),
                name: e.shortcode().map_or_else(|| e.name().replace(' ', "_"), str::to_owned),
                section: e.group() as usize,
            })
            .collect(),
        Source::Reactions => reactions::all()
            .map(|(kind, name, glyph)| Item {
                key: kind.to_string(),
                glyph,
                name: name.to_lowercase(),
                section: 0,
            })
            .collect(),
    }
}

fn storage_key(source: Source) -> &'static str {
    match source {
        Source::Text => "mostik.emoji.recent",
        Source::Reactions => "mostik.reactions.recent",
    }
}

fn load_recent(source: Source) -> Vec<String> {
    let saved = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(storage_key(source)).ok().flatten())
        .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok());
    match (saved, source) {
        (Some(v), _) => v,
        // Before the first pick: the default Telemost set / popular emoji, like Slack's starter set.
        (None, Source::Reactions) => reactions::DEFAULT_SET.iter().map(u64::to_string).collect(),
        (None, Source::Text) => ["👍", "😄", "🎉", "❤️", "🙏", "👀", "🔥", "✅", "😂"]
            .map(str::to_owned)
            .to_vec(),
    }
}

/// Puts the picked emoji first in "Frequently used".
pub fn remember(source: Source, key: &str) {
    let mut list = load_recent(source);
    list.retain(|k| k != key);
    list.insert(0, key.to_owned());
    list.truncate(RECENT_MAX);
    if let (Some(s), Ok(v)) = (
        web_sys::window().and_then(|w| w.local_storage().ok().flatten()),
        serde_json::to_string(&list),
    ) {
        let _ = s.set_item(storage_key(source), &v);
    }
}

/// Suggestions for `:code` in the composer, like Slack's emoji autocomplete.
pub fn suggest(query: &str, limit: usize) -> Vec<(&'static str, String)> {
    let q = query.to_lowercase();
    let mut starts = vec![];
    let mut contains = vec![];
    for e in emojis::iter() {
        for code in e.shortcodes() {
            if code.starts_with(&q) {
                starts.push((e.as_str(), code.to_owned()));
                break;
            } else if code.contains(&q) {
                contains.push((e.as_str(), code.to_owned()));
                break;
            }
        }
        if starts.len() >= limit {
            break;
        }
    }
    starts.extend(contains);
    starts.truncate(limit);
    starts
}

/// Anchor point of the picker: the top right corner of the clicked button.
pub fn anchor_of(e: &leptos::ev::MouseEvent) -> (f64, f64) {
    e.current_target()
        .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
        .map_or((e.client_x() as f64, e.client_y() as f64), |el| {
            let r = el.get_bounding_client_rect();
            (r.right(), r.top())
        })
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
) -> impl IntoView {
    let all = StoredValue::new(items(source));
    let recent = load_recent(source);
    let query = RwSignal::new(String::new());
    let hover: RwSignal<Option<(&'static str, String)>> = RwSignal::new(None);
    let body = NodeRef::<html::Div>::new();
    let search = NodeRef::<html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = search.get() {
            let _ = el.focus();
        }
    });

    let sections: Vec<&'static str> = match source {
        Source::Text => tr().emoji_groups.to_vec(),
        Source::Reactions => vec![tr().all_reactions],
    };
    let tabs: Vec<&'static str> = match source {
        Source::Text => TEXT_TABS.to_vec(),
        Source::Reactions => vec!["😄"],
    };
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
        let (key, glyph, name) = (it.key.clone(), it.glyph, it.name.clone());
        view! {
            <button class="ep-cell" on:mouseenter=move |_| hover.set(Some((glyph, name.clone()))) on:click=move |_| pick(key.clone())>{glyph}</button>
        }
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
        <div class="emoji-picker" style=place(anchor) on:click=|e| e.stop_propagation()>
            <div class="ep-search">
                <input node_ref=search class="text-input" placeholder=match source { Source::Text => tr().search_emoji, Source::Reactions => tr().search_reactions }
                    prop:value=move || query.get()
                    on:input=move |e| query.set(event_target_value(&e))
                    on:keydown=move |e| if e.key() == "Escape" { e.stop_propagation(); on_close.run(()); }/>
            </div>
            <div class="ep-tabs">
                <button title=tr().frequently_used on:click=move |_| jump("recent".into())>"🕘"</button>
                {tabs.into_iter().enumerate().map(|(i, t)| {
                    let title = match source { Source::Text => tr().emoji_groups[i], Source::Reactions => tr().all_reactions };
                    view! { <button title=title on:click=move |_| jump(i.to_string())>{t}</button> }
                }).collect_view()}
            </div>
            <div class="ep-body" node_ref=body>{grid}</div>
            <div class="ep-foot">
                {move || match hover.get() {
                    Some((glyph, name)) => view! { <span class="ep-big">{glyph}</span><span class="ep-name">{format!(":{name}:")}</span> }.into_any(),
                    None => view! { <span class="ep-big">"🙂"</span><span class="ep-name muted">{tr().pick_emoji}</span> }.into_any(),
                }}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_prefers_prefix() {
        let s = suggest("thumbs", 8);
        assert_eq!(s.first().map(|x| x.0), Some("👍"));
        assert!(suggest("tada", 8).iter().any(|x| x.0 == "🎉"));
        assert!(suggest("zzzzqq", 8).is_empty());
    }

    #[test]
    fn groups_match_labels() {
        assert_eq!(emojis::Group::iter().count(), TEXT_TABS.len());
        assert_eq!(
            crate::i18n::for_lang(crate::i18n::Lang::En).emoji_groups.len(),
            TEXT_TABS.len()
        );
    }
}
