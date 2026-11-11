//! Own sidebar sections ("folders"), as custom sections in Slack. Telemost has nothing like them,
//! so they live in `localStorage`. A chat put into a folder leaves its own section; a starred
//! chat stays in "Starred", which Telemost keeps.

use mostik_core::{Chat, ChatId, ChatKind, Section};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    /// Emoji before the name, as in Slack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Chats from top to bottom.
    pub chats: Vec<ChatId>,
}

pub type Folders = Vec<Folder>;

const KEY: &str = "mostik.folders";
const PREFIX: &str = "folder:";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn load() -> Folders {
    storage()
        .and_then(|s| s.get_item(KEY).ok().flatten())
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

pub fn save(folders: &Folders) {
    if let (Some(s), Ok(v)) = (storage(), serde_json::to_string(folders)) {
        let _ = s.set_item(KEY, &v);
    }
}

pub fn is_folder(section_id: &str) -> bool {
    section_id.starts_with(PREFIX)
}

/// An id no folder has yet.
pub fn new_id(folders: &Folders, seed: u64) -> String {
    (seed..)
        .map(|n| format!("{PREFIX}{n:x}"))
        .find(|id| !folders.iter().any(|f| &f.id == id))
        .unwrap_or_default()
}

/// The Telemost section a chat belongs to by its kind.
pub fn home_section(ch: &Chat) -> &'static str {
    match ch.kind {
        ChatKind::Direct { .. } | ChatKind::Notes => "dms",
        _ if ch.id.0.starts_with("0/22/") || ch.id.0.starts_with("0/32/") => "meetings",
        _ => "channels",
    }
}

/// The folder's name with its emoji, for menus.
pub fn label(f: &Folder) -> String {
    match &f.emoji {
        Some(e) => format!("{e} {}", f.name),
        None => f.name.clone(),
    }
}

/// The folder the chat is in.
pub fn folder_of<'a>(folders: &'a Folders, chat: &ChatId) -> Option<&'a Folder> {
    folders.iter().find(|f| f.chats.contains(chat))
}

/// The sidebar sections: "Starred", then the folders in their order, then the Telemost sections
/// without the chats taken into folders. Folders show only chats the sections know.
pub fn arrange(sections: &[Section], folders: &Folders) -> Vec<Section> {
    let starred: &[ChatId] = sections
        .iter()
        .find(|s| s.id == "starred")
        .map_or(&[], |s| s.chats.as_slice());
    let known = |id: &ChatId| sections.iter().any(|s| s.id != "starred" && s.chats.contains(id));
    let filed = |id: &ChatId| !starred.contains(id) && folder_of(folders, id).is_some();
    let mut out: Vec<Section> = sections.iter().filter(|s| s.id == "starred").cloned().collect();
    out.extend(folders.iter().map(|f| {
        Section {
            id: f.id.clone(),
            title: f.name.clone(),
            chats: f
                .chats
                .iter()
                .filter(|id| !starred.contains(id) && known(id))
                .cloned()
                .collect(),
        }
    }));
    out.extend(sections.iter().filter(|s| s.id != "starred").map(|s| Section {
        id: s.id.clone(),
        title: s.title.clone(),
        chats: s.chats.iter().filter(|id| !filed(id)).cloned().collect(),
    }));
    out
}

/// Takes the chat out of any folder and puts it into folder `to` (nowhere with `None`): next to
/// `anchor` (after it with `true`) or at the end.
pub fn move_chat(folders: &mut Folders, chat: &ChatId, to: Option<&str>, anchor: Option<(&ChatId, bool)>) {
    for f in folders.iter_mut() {
        f.chats.retain(|id| id != chat);
    }
    let Some(f) = to.and_then(|to| folders.iter_mut().find(|f| f.id == to)) else {
        return;
    };
    let at = anchor
        .and_then(|(a, after)| f.chats.iter().position(|id| id == a).map(|i| i + usize::from(after)))
        .unwrap_or(f.chats.len());
    f.chats.insert(at, chat.clone());
}

/// Puts the folder's chats in the `shown` order; chats it has but does not show stay at the end.
pub fn reorder(folders: &mut Folders, id: &str, shown: &[ChatId]) {
    if let Some(f) = folders.iter_mut().find(|f| f.id == id) {
        let rest: Vec<ChatId> = f.chats.iter().filter(|c| !shown.contains(c)).cloned().collect();
        f.chats = shown.iter().cloned().chain(rest).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<ChatId> {
        list.iter().map(|s| ChatId((*s).into())).collect()
    }

    fn section(id: &str, chats: &[&str]) -> Section {
        Section {
            id: id.into(),
            title: id.into(),
            chats: ids(chats),
        }
    }

    fn folder(id: &str, chats: &[&str]) -> Folder {
        Folder {
            id: id.into(),
            name: id.to_uppercase(),
            emoji: None,
            chats: ids(chats),
        }
    }

    #[test]
    fn folders_take_chats_out_of_their_sections() {
        let sections = vec![
            section("starred", &["s"]),
            section("channels", &["a", "b", "c"]),
            section("dms", &["d"]),
        ];
        // "s" is starred and stays there; "gone" is unknown and is not shown.
        let folders = vec![folder("folder:1", &["c", "s", "gone", "d"]), folder("folder:2", &[])];
        let out = arrange(&sections, &folders);
        let got: Vec<(&str, Vec<ChatId>)> = out.iter().map(|s| (s.id.as_str(), s.chats.clone())).collect();
        assert_eq!(
            got,
            vec![
                ("starred", ids(&["s"])),
                ("folder:1", ids(&["c", "d"])),
                ("folder:2", vec![]),
                ("channels", ids(&["a", "b"])),
                ("dms", vec![]),
            ]
        );
        assert_eq!(out[1].title, "FOLDER:1");
        assert_eq!(arrange(&sections, &vec![]), sections);
    }

    #[test]
    fn chat_moves_between_folders() {
        let mut folders = vec![folder("folder:1", &["a", "b"]), folder("folder:2", &["c"])];
        let a = ChatId("a".into());
        let c = ChatId("c".into());
        move_chat(&mut folders, &a, Some("folder:2"), Some((&c, false)));
        assert_eq!(folders[0].chats, ids(&["b"]));
        assert_eq!(folders[1].chats, ids(&["a", "c"]));
        move_chat(&mut folders, &a, Some("folder:2"), Some((&c, true)));
        assert_eq!(folders[1].chats, ids(&["c", "a"]));
        move_chat(&mut folders, &a, Some("folder:1"), None);
        assert_eq!(folders[0].chats, ids(&["b", "a"]));
        reorder(&mut folders, "folder:1", &ids(&["a"]));
        assert_eq!(folders[0].chats, ids(&["a", "b"]));
        move_chat(&mut folders, &a, None, None);
        assert!(folder_of(&folders, &a).is_none());
        // "folder:1" and "folder:2" are taken.
        assert_eq!(new_id(&folders, 1), "folder:3");
        assert_eq!(new_id(&vec![], 1), "folder:1");
    }
}
