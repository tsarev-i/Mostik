//! Notifications: which messages notify, mentions of the user, the notification itself with its
//! icon and sound.

use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;

use mostik_core::event::Alert;
use mostik_core::{Chat, ChatId, Message, NotifyLevel, Timestamp, User, UserId};
use mostik_ipc::cmd::{self, Args, Out, SoundArgs};
use mostik_yandex::clock;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use super::{AppState, NotifyMode, Shared, text};

/// Runs `f` with the people, public chats and ids `@` can point at.
pub(super) fn with_names<R>(sh: &Shared, f: impl FnOnce(&mostik_core::markup::Mentionable) -> R) -> R {
    let people: Vec<&str> = sh.names.values().map(String::as_str).collect();
    let channels: Vec<&str> = sh.channels.iter().map(String::as_str).collect();
    let ids: Vec<(&str, &str)> = sh.names.iter().map(|(i, n)| (i.as_str(), n.as_str())).collect();
    f(&mostik_core::markup::Mentionable {
        people: &people,
        channels: &channels,
        ids: &ids,
    })
}

/// Whether message `m` mentions the user: by id, by name, through a public chat or `@all`.
pub(super) fn mentions_me(sh: &Shared, m: &Message) -> bool {
    sh.me.as_ref().is_some_and(|me| {
        let me = mostik_core::markup::Me {
            id: &me.0,
            name: sh.names.get(&me.0).map_or("", String::as_str),
        };
        with_names(sh, |names| mostik_core::markup::mentions_me(&m.text, names, me))
    })
}

/// How long a connection may have been dead before Mostik noticed it: Xiva pings once a minute,
/// a connection silent for one and a half of that is dropped, checked every 10 s; a reset
/// comes with the next heartbeat, a minute apart.
const NOTICED_LATE_MCS: Timestamp = 3 * 60 * 1_000_000;

/// What a connection that went down had seen: the last message of each chat (`Shared::last`)
/// and when it was found closed. Pushes are lost while a connection is dead and until the next
/// one is up, and their messages come with its chat list instead, which notifies of nothing:
/// [`missed`] finds them there. Telemost drops the connection now and then, in storms on a
/// shaky network.
pub(super) struct Gap {
    pub generation: u64,
    pub since: Timestamp,
    pub last: BTreeMap<String, (Timestamp, u64)>,
}

/// The messages to notify of after a reconnection: the last message of each chat in the new chat
/// list (`messages`, one per chat) that the connection before had not seen, sent by someone else
/// at most a few minutes before the drop was noticed, in a chat still unread (not read
/// meanwhile on another device). Thread replies and notes about the chat are left out, as in
/// [`notify`]; whether each notifies is up to it.
pub(super) fn missed(
    gap: &Gap,
    chats: &[Chat],
    messages: &[Message],
    alerts: &BTreeMap<(String, Timestamp), Alert>,
    me: Option<&UserId>,
) -> Vec<(Message, Alert)> {
    let unread = |chat: &ChatId| chats.iter().any(|c| &c.id == chat && c.unread > 0);
    let mut out: Vec<(Message, Alert)> = Vec::new();
    for m in messages {
        let seen = gap.last.get(&m.chat_id.0).map_or(0, |l| l.0);
        if m.ts <= seen
            || m.ts.saturating_add(NOTICED_LATE_MCS) < gap.since
            || m.thread_parent.is_some()
            || m.is_note()
            || m.edited
            || me == Some(&m.author)
            || !unread(&m.chat_id)
        {
            continue;
        }
        let alert = alerts.get(&(m.chat_id.0.clone(), m.ts)).copied().unwrap_or_default();
        match out.iter_mut().find(|(o, _)| o.chat_id == m.chat_id) {
            Some(o) if o.0.ts < m.ts => *o = (m.clone(), alert),
            Some(_) => {}
            None => out.push((m.clone(), alert)),
        }
    }
    out.sort_by_key(|(m, _)| m.ts);
    out
}

/// Whether notifications are paused now.
fn paused(sh: &Shared) -> bool {
    sh.paused.is_some_and(|p| p.active_at(clock::now_mcs()))
}

/// Whether a call ringing in `m`'s chat may ring: not in "Do not disturb", not in a chat that
/// notifies about nothing, not with notifications off. Unlike messages, a call rings even when
/// the window is in focus, as in Slack.
pub(super) fn may_ring(app: &AppHandle, shared: &StdMutex<Shared>, m: &Message) -> bool {
    let Ok(sh) = shared.lock() else { return false };
    if !sh.ready
        || sh.me.as_ref() == Some(&m.author)
        || paused(&sh)
        || sh.muted.get(&m.chat_id.0) == Some(&NotifyLevel::Nothing)
    {
        return false;
    }
    let state = app.state::<AppState>();
    state.prefs.lock().is_ok_and(|p| p.notify != NotifyMode::Nothing)
}

/// Notifies of message `m`, unless something says not to; with a sound unless its sender asked for
/// none (`alert`).
pub(super) fn notify(app: &AppHandle, shared: &StdMutex<Shared>, m: &Message, author: Option<&User>, alert: Alert) {
    let Ok(mut sh) = shared.lock() else { return };
    // A thread reply lives in the thread chat: its time and number mean nothing in the parent chat.
    // A ringing call is not a message in the chat: the read marker cannot point at it.
    if m.thread_parent.is_none() && !m.is_ringing() {
        let entry = sh.last.entry(m.chat_id.0.clone()).or_default();
        if m.ts > entry.0 {
            *entry = (m.ts, m.seq_no.max(entry.1));
        }
    }
    // Only the start of a call notifies; "the call ended" and changes of the chat are just lines in it.
    let ended = matches!(m.call, Some(mostik_core::CallNote::Ended { .. }));
    if !sh.ready
        || sh.me.as_ref() == Some(&m.author)
        || m.edited
        || ended
        || m.system.is_some()
        || alert == Alert::Silent
    {
        return;
    }
    // A muted chat notifies about mentions only, or about nothing.
    match sh.muted.get(&m.chat_id.0) {
        Some(NotifyLevel::Nothing) => return,
        Some(NotifyLevel::Mentions) if !mentions_me(&sh, m) => return,
        _ => {}
    }
    if paused(&sh) {
        return;
    }
    let prefs = app
        .state::<AppState>()
        .prefs
        .lock()
        .map(|p| p.clone())
        .unwrap_or_default();
    let direct = !m.chat_id.0.contains('/');
    // Only "direct and mentions" needs to know about mentions.
    let mentioned = || mentions_me(&sh, m);
    match prefs.notify {
        NotifyMode::Nothing => return,
        NotifyMode::Direct if !direct && !mentioned() => return,
        _ => {}
    }
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if focused {
        return;
    }
    let who = author
        .map(|a| a.display_name.clone())
        .or_else(|| sh.names.get(&m.author.0).cloned())
        .unwrap_or_else(|| text(app, "Новое сообщение", "New message").into());
    let title = match sh.titles.get(&m.chat_id.0) {
        Some(chat) if chat != &who => format!("{who} · {chat}"),
        _ => who,
    };
    let voice = m.media.as_ref().and_then(mostik_core::Media::voice);
    let body: String = if m.call.is_some() {
        text(app, "📞 Начался созвон", "📞 A call started").to_owned()
    } else if matches!(m.media, Some(mostik_core::Media::Sticker(_))) {
        text(app, "Стикер", "Sticker").to_owned()
    } else if let Some(v) = voice.filter(|_| m.text.is_empty()) {
        match prefs.preview && v.recognized && !v.text.is_empty() {
            true => format!("🎤 {}", v.text.chars().take(200).collect::<String>()),
            false => text(app, "🎤 Голосовое сообщение", "🎤 Voice message").to_owned(),
        }
    } else if prefs.preview {
        with_names(&sh, |names| mostik_core::markup::plain(&m.text, names))
            .chars()
            .take(200)
            .collect()
    } else {
        text(app, "Новое сообщение", "New message").to_owned()
    };
    let body = if body.is_empty() && !m.attachments.is_empty() {
        text(app, "📎 Вложение", "📎 Attachment").to_owned()
    } else {
        body
    };
    // A click on the notification opens this message (see `open_from_notification`).
    let mut builder = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .extra("chat", &m.chat_id.0)
        .extra("ts", m.ts);
    if let Some(icon) = notification_icon(app) {
        builder = builder.icon(icon);
    }
    if let Some(parent) = m.thread_parent {
        builder = builder.extra("thread", parent);
    }
    if let Err(e) = builder.show() {
        eprintln!("notification not shown: {e}");
    }
    if alert != Alert::Quiet {
        crate::sound::play(&prefs.sound);
    }
}

/// Plays a notification sound: the preview in the preferences.
#[tauri::command]
pub fn play_sound(args: Args<cmd::PlaySound>) -> Out<cmd::PlaySound> {
    let SoundArgs { id } = args;
    crate::sound::play(&id);
}

/// Icon of the notifications, as a file path. Without one the notification plugin gives the name
/// of the executable as the icon, which no icon theme has, and Plasma draws a broken image next
/// to the text. The app icon is written to the cache once per run.
fn notification_icon(app: &AppHandle) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        PATH.get_or_init(|| {
            const ICON: &[u8] = include_bytes!("../../icons/128x128@2x.png");
            let dir = app.path().app_cache_dir().ok()?;
            let path = dir.join("notification-icon.png");
            if std::fs::read(&path).ok().as_deref() != Some(ICON) {
                std::fs::create_dir_all(&dir).ok()?;
                std::fs::write(&path, ICON).ok()?;
            }
            path.to_str().map(str::to_owned)
        })
        .clone()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        None
    }
}

/// "Show an example" in the notification preferences.
#[tauri::command]
pub fn test_notification(app: AppHandle) -> Result<Out<cmd::TestNotification>, String> {
    let mut builder = app
        .notification()
        .builder()
        .title(text(&app, "Мостик", "Mostik"))
        .body(text(
            &app,
            "Так выглядит уведомление о новом сообщении.",
            "This is what a new message notification looks like.",
        ));
    if let Some(icon) = notification_icon(&app) {
        builder = builder.icon(icon);
    }
    builder.show().map_err(|e| {
        format!(
            "{}: {e}",
            text(
                &app,
                "система не показала уведомление",
                "the system didn't show the notification"
            )
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mostik_core::ChatKind;

    const MIN: Timestamp = 60 * 1_000_000;

    fn chat(id: &str, unread: u32) -> Chat {
        Chat {
            unread,
            ..Chat::new(ChatId(id.into()), ChatKind::Group { public: false })
        }
    }

    fn msg(chat: &str, ts: Timestamp, author: &str) -> Message {
        Message {
            chat_id: ChatId(chat.into()),
            ts,
            author: UserId(author.into()),
            text: "hi".into(),
            thread_parent: None,
            edited: false,
            important: false,
            reactions: vec![],
            thread: None,
            seq_no: 0,
            attachments: vec![],
            forwarded: vec![],
            call: None,
            system: None,
            media: None,
            reads: None,
        }
    }

    /// After a reconnection, the last message of each chat the dropped connection had not seen
    /// notifies, as its sender wanted; old ones, own ones and ones read meanwhile do not.
    #[test]
    fn messages_missed_while_the_connection_was_down() {
        let since = 100 * MIN;
        let gap = Gap {
            generation: 1,
            since,
            last: BTreeMap::from([("a".into(), (since - 5 * MIN, 1)), ("b".into(), (since - MIN, 1))]),
        };
        let me = UserId("me".into());
        let chats = [
            chat("a", 1),
            chat("b", 1),
            chat("c", 1),
            chat("d", 0),
            chat("e", 2),
            chat("f", 1),
        ];
        let messages = [
            // Came while the connection was down.
            msg("a", since - 2 * MIN, "anna"),
            // Seen by the connection before.
            msg("b", since - MIN, "boris"),
            // A chat new to the list, long before the drop.
            msg("c", since - 60 * MIN, "anna"),
            // Read meanwhile on another device.
            msg("d", since + MIN, "anna"),
            // Sent by the user from another device.
            msg("e", since + MIN, "me"),
            // Sent without any notification.
            msg("f", since + 2 * MIN, "vera"),
        ];
        let alerts = BTreeMap::from([(("f".to_owned(), since + 2 * MIN), Alert::Silent)]);
        let found = missed(&gap, &chats, &messages, &alerts, Some(&me));
        let found: Vec<_> = found.iter().map(|(m, a)| (m.chat_id.0.as_str(), *a)).collect();
        assert_eq!(found, [("a", Alert::Default), ("f", Alert::Silent)]);
    }
}
