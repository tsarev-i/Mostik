//! Emoji around the picker (in `picker`): what it offers (text or reactions), its
//! "Frequently used" list, its anchor, and the `:code` suggestions of the composer.

mod picker;

use mostik_core::reactions;

pub use picker::EmojiPicker;

/// How many emoji to remember in "Frequently used".
const RECENT_MAX: usize = 27;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Any Unicode emoji: inserted into the message text.
    Text,
    /// Telemost reactions: the server accepts only types `100101…100157`.
    Reactions,
}

fn storage_key(source: Source) -> &'static str {
    match source {
        Source::Text => "mostik.emoji.recent",
        Source::Reactions => "mostik.reactions.recent",
    }
}

fn load_recent(source: Source) -> Vec<String> {
    let saved = crate::storage::load::<Vec<String>>(storage_key(source));
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
    crate::storage::save(storage_key(source), &list);
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
}
