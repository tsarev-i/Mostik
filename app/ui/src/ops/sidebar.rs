//! The sidebar and the chat menu: sections (Telemost folders), hidden chats, notification
//! levels and starred chats.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::Ctx;
use crate::bridge;
use crate::i18n::tr;

use super::failed;
use super::snapshot::sort_sections;

/// Changes the sidebar sections of the current organization and writes them to Telemost, where
/// they are chat folders. Shown at once; put back if Telemost refuses.
pub fn change_folders(c: Ctx, change: impl FnOnce(&mut crate::folders::Folders)) {
    use crate::folders;
    let before = c.folders.get_untracked();
    let mut list = before.clone();
    change(&mut list);
    if list == before {
        return;
    }
    let org = c.snap.with_untracked(|s| crate::app::org_key(s, c.org.get_untracked()));
    let put = move |list: &folders::Folders| {
        let mine: Vec<_> = list.iter().map(|f| folders::to_telemost(f, &org)).collect();
        c.snap.update(|s| {
            s.folders.retain(|f| f.org != org);
            s.folders.extend(mine.iter().cloned());
        });
        mine
    };
    let mine = put(&list);
    if !c.live {
        return;
    }
    let org = c.snap.with_untracked(|s| crate::app::org_key(s, c.org.get_untracked()));
    spawn_local(async move {
        if let Err(e) = bridge::save_chat_folders(&org, &mine).await {
            put(&before);
            failed(c, tr().err_settings, e);
        }
    });
}

/// Hides the chat from the sidebar until something new comes in it.
pub fn hide_chat(c: Ctx, id: ChatId) {
    let Some(last) = c.snap.with_untracked(|s| s.chat(&id).map(|ch| ch.last_ts)) else {
        return;
    };
    c.hidden.update(|h| {
        h.insert(id, last);
    });
}

/// What a chat notifies about: everything, mentions only or nothing.
pub fn set_notify_level(c: Ctx, id: ChatId, level: NotifyLevel) {
    spawn_local(async move {
        if c.live
            && let Err(e) = bridge::set_notify_level(&id.0, level).await
        {
            return failed(c, tr().err_settings, e);
        }
        c.snap.update(|s| {
            if let Some(ch) = s.chats.iter_mut().find(|ch| ch.id == id) {
                ch.set_notify_level(level);
            }
        });
        c.notice.set(Some(tr().notify_level_notice(level)));
    });
}

pub fn is_starred(c: Ctx, id: &ChatId) -> bool {
    c.snap.with_untracked(|s| {
        s.sections
            .iter()
            .any(|sec| sec.id == "starred" && sec.chats.contains(id))
    })
}

/// "Star" / "Unstar": pinned chats in Telemost.
pub fn toggle_star(c: Ctx, id: ChatId) {
    let starred = !is_starred(c, &id);
    spawn_local(async move {
        let list: Vec<ChatId> = if c.live {
            match bridge::set_starred(&id.0, starred).await {
                Ok(list) => list.into_iter().map(ChatId).collect(),
                Err(e) => return failed(c, tr().err_settings, e),
            }
        } else {
            let mut list = c.snap.with_untracked(|s| {
                s.sections
                    .iter()
                    .find(|sec| sec.id == "starred")
                    .map(|sec| sec.chats.clone())
                    .unwrap_or_default()
            });
            list.retain(|x| x != &id);
            if starred {
                list.push(id.clone());
            }
            list
        };
        c.snap.update(|s| apply_starred(s, list));
    });
}

/// Sorts chats into sections: starred ones into "Starred", the rest by kind.
pub(super) fn apply_starred(s: &mut Snapshot, starred: Vec<ChatId>) {
    let starred: Vec<ChatId> = starred.into_iter().filter(|id| s.chat(id).is_some()).collect();
    let old: Vec<ChatId> = s
        .sections
        .iter()
        .find(|sec| sec.id == "starred")
        .map(|sec| sec.chats.clone())
        .unwrap_or_default();
    s.sections.retain(|sec| sec.id != "starred");
    for sec in &mut s.sections {
        sec.chats.retain(|id| !starred.contains(id));
    }
    // Unstarred chats return to their own section.
    for id in old.iter().filter(|id| !starred.contains(id)) {
        let Some(ch) = s.chat(id) else { continue };
        let target = ch.home_section();
        if let Some(sec) = s.sections.iter_mut().find(|sec| sec.id == target)
            && !sec.chats.contains(id)
        {
            sec.chats.push(id.clone());
        }
    }
    if !starred.is_empty() {
        let title = tr().section("starred", "").to_owned();
        s.sections.insert(
            0,
            Section {
                id: "starred".into(),
                title,
                chats: starred,
            },
        );
    }
    sort_sections(s);
}
