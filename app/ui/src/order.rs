//! Custom conversation order in the sidebar sections: the user drags chats up and down,
//! the order of each section is kept in `localStorage`.

use std::collections::HashMap;

use mostik_core::ChatId;

/// Saved order: section id → its chats from top to bottom.
pub type ChatOrder = HashMap<String, Vec<ChatId>>;

const KEY: &str = "mostik.chat_order";

pub fn load() -> ChatOrder {
    crate::storage::load(KEY).unwrap_or_default()
}

pub fn save(order: &ChatOrder) {
    crate::storage::save(KEY, order);
}

/// Puts `base` into the `saved` order. Chats missing from `saved` (new ones) keep their `base` order
/// and go on top, or at the bottom with `new_last`.
pub fn apply(saved: &[ChatId], base: &[ChatId], new_last: bool) -> Vec<ChatId> {
    let (mut known, fresh): (Vec<&ChatId>, Vec<&ChatId>) = base.iter().partition(|id| saved.contains(id));
    known.sort_by_key(|id| saved.iter().position(|s| s == *id));
    let (first, second) = if new_last { (known, fresh) } else { (fresh, known) };
    first.into_iter().chain(second).cloned().collect()
}

/// Moves `chat` next to `anchor`: right after it with `after`, otherwise right before it.
pub fn move_chat(mut list: Vec<ChatId>, chat: &ChatId, anchor: &ChatId, after: bool) -> Vec<ChatId> {
    if chat == anchor || !list.contains(chat) {
        return list;
    }
    list.retain(|id| id != chat);
    let Some(at) = list.iter().position(|id| id == anchor) else {
        return list;
    };
    list.insert(at + usize::from(after), chat.clone());
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<ChatId> {
        list.iter().map(|s| ChatId((*s).into())).collect()
    }

    #[test]
    fn apply_keeps_saved_order_and_puts_new_chats_on_top() {
        let saved = ids(&["c", "a", "gone"]);
        let base = ids(&["a", "n1", "c", "n2"]);
        assert_eq!(apply(&saved, &base, false), ids(&["n1", "n2", "c", "a"]));
        assert_eq!(apply(&saved, &base, true), ids(&["c", "a", "n1", "n2"]));
        assert_eq!(apply(&[], &base, false), base);
    }

    #[test]
    fn move_chat_before_and_after_anchor() {
        let list = ids(&["a", "b", "c", "d"]);
        assert_eq!(
            move_chat(list.clone(), &ChatId("d".into()), &ChatId("b".into()), false),
            ids(&["a", "d", "b", "c"])
        );
        assert_eq!(
            move_chat(list.clone(), &ChatId("a".into()), &ChatId("c".into()), true),
            ids(&["b", "c", "a", "d"])
        );
        assert_eq!(
            move_chat(list.clone(), &ChatId("a".into()), &ChatId("d".into()), true),
            ids(&["b", "c", "d", "a"])
        );
        assert_eq!(
            move_chat(list.clone(), &ChatId("b".into()), &ChatId("b".into()), true),
            list
        );
        assert_eq!(
            move_chat(list.clone(), &ChatId("x".into()), &ChatId("b".into()), true),
            list
        );
    }
}
