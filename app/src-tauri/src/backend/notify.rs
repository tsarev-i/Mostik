//! Notifications: which messages notify, mentions of the user, the notification itself with its
//! icon and sound.

use std::sync::Mutex as StdMutex;

use mostik_core::{Message, NotifyLevel, User};
use mostik_yandex::clock;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use super::{AppState, NotifyMode, Shared, text};

/// Runs `f` with the people, public chats and ids `@` can point at.
fn with_names<R>(sh: &Shared, f: impl FnOnce(&mostik_core::markup::Mentionable) -> R) -> R {
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

pub(super) fn notify(app: &AppHandle, shared: &StdMutex<Shared>, m: &Message, author: Option<&User>) {
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
    if !sh.ready || sh.me.as_ref() == Some(&m.author) || m.edited || ended || m.system.is_some() {
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
    let body: String = if m.call.is_some() {
        text(app, "📞 Начался созвон", "📞 A call started").to_owned()
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
    crate::sound::play(&prefs.sound);
}

/// Plays a notification sound: the preview in the preferences.
#[tauri::command]
pub fn play_sound(id: String) {
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
pub fn test_notification(app: AppHandle) -> Result<(), String> {
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
