//! Calls: starting one in a chat or with a person, bringing its window back, leaving it.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::Ctx;
use crate::bridge;
use crate::i18n::tr;

use super::{failed, report};

/// A call in a chat, like a Slack huddle: the chat meeting opens in the call window.
/// If this chat's call is already open, its window comes to the front.
pub fn start_call(c: Ctx, chat: ChatId) {
    if !c.live {
        return report(c, tr().only_in_app_calls.into());
    }
    if let Some(current) = c.call.get_untracked() {
        // One call at a time, as in Slack: bring the current one back instead.
        focus_call(c);
        if current.chat_id.as_deref() != Some(chat.0.as_str()) {
            report(c, tr().call_busy.into());
        }
        return;
    }
    let label = c.snap.with_untracked(|s| {
        s.chat(&chat).map_or_else(String::new, |ch| {
            let title = crate::app::display_title(s, ch);
            match ch.kind {
                ChatKind::Group { public: true } | ChatKind::Broadcast => format!("#{title}"),
                _ => title,
            }
        })
    });
    // The first call of a group call chat has no link in its notes: Telemost's report of the call
    // has it, or `create_meeting` finds it.
    let url = c
        .active_calls
        .with_untracked(|a| a.get(&chat).cloned())
        .and_then(|url| {
            if url.is_empty() {
                c.meetings
                    .with_untracked(|m| m.get(&chat).filter(|m| !m.ended).map(|m| m.url.clone()))
            } else {
                Some(url)
            }
        });
    c.notice.set(Some(tr().call_opening.into()));
    spawn_local(async move {
        if let Err(e) = bridge::start_call(&chat.0, &label, url.as_deref()).await {
            c.notice.set(None);
            failed(c, tr().err_call, e);
        }
    });
}

/// The call button of a chat whose call is already open: bring its window back.
pub fn focus_call(c: Ctx) {
    spawn_local(async move {
        if let Err(e) = bridge::focus_call().await {
            failed(c, tr().err_call, e);
        }
    });
}

/// "Leave" in the sidebar call bar: the bar goes away when the backend reports the call closed.
pub fn leave_call(c: Ctx) {
    spawn_local(async move {
        if let Err(e) = bridge::leave_call().await {
            failed(c, tr().err_call, e);
        }
    });
}

/// "Call" in a profile: a meeting in the direct chat with the person (the chat is created if needed).
pub fn call_user(c: Ctx, user: UserId) {
    if !c.live {
        return report(c, tr().only_in_app_calls.into());
    }
    let existing = c.snap.with_untracked(|s| {
        s.chats
            .iter()
            .find(|ch| {
                matches!(&ch.kind, ChatKind::Direct { peer } if peer == &user)
                    || (ch.kind == ChatKind::Notes && user == s.me)
            })
            .map(|ch| ch.id.clone())
    });
    if let Some(id) = existing {
        return start_call(c, id);
    }
    spawn_local(async move {
        match bridge::open_dm(&user.0).await {
            Ok(chat) => start_call(c, chat.id),
            Err(e) => failed(c, tr().err_open_dm, e),
        }
    });
}
