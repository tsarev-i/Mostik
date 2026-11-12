//! Live events from the socket: diagnostics, `Shared` updates, events for the UI, calls ringing,
//! chats that arrive with their first message, and the names of the people mentioned.

use std::io::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use mostik_core::{CallNote, Chat, ChatKind, Message, User, UserId};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::shape;
use mostik_yandex::xiva_client::Incoming;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use super::chats::chat_info_raw;
use super::connection::{probing, reconnect_loop};
use super::notify::{may_ring, notify};
use super::threads::reply_notifies_now;
use super::{AppState, EVENT, Live, Shared, fill_profiles, live, service};

/// Socket events written to `probe-events.jsonl` per run.
const MAX_DIAG_EVENTS: usize = 300;

/// With `MOSTIK_PUSH_LOG=<file>` every push goes to the traffic log as is, with `parsed`:
/// whether the app understood it.
fn log_push(incoming: &Incoming, me: Option<&UserId>) {
    if !mostik_yandex::diag::traffic_logging() {
        return;
    }
    mostik_yandex::diag::traffic_log(match incoming {
        Incoming::Push { event, body } => {
            json!({ "event": event, "parsed": convert::from_push(body, me).is_some(), "body": body })
        }
        Incoming::Text(_) => return,
        Incoming::Closed => json!({ "closed": true }),
    });
}

/// Socket event of a connection of session `generation`: diagnostics, `Shared` update, UI event,
/// notification.
pub(super) fn handle_incoming(
    app: &AppHandle,
    shared: &Arc<StdMutex<Shared>>,
    diag: &AtomicUsize,
    generation: u64,
    incoming: Incoming,
) {
    log_push(&incoming, shared.lock().ok().and_then(|sh| sh.me.clone()).as_ref());
    match incoming {
        Incoming::Push { event, body } => {
            if probing()
                && diag.fetch_add(1, Ordering::SeqCst) < MAX_DIAG_EVENTS
                && let Ok(dir) = app.path().app_data_dir()
                && let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join("probe-events.jsonl"))
            {
                let _ = writeln!(f, "{}", json!({ "event": event, "shape": shape(&body) }));
            }
            let ringing = convert::ringing(&body);
            if let Some(convert::Ringing::Ended { id }) = &ringing {
                crate::ring::stop(app, id);
                return;
            }
            let me = shared.lock().ok().and_then(|sh| sh.me.clone());
            let Some(ev) = convert::from_push(&body, me.as_ref()) else {
                return;
            };
            if service::swallows(app, shared, &ev) {
                return;
            }
            if !sync_shared(app, shared, &ev) {
                return;
            }
            // A call rings with a dialog in the main window and a ringtone instead of a notification.
            if let LiveEvent::Upsert { message, author, .. } = &ev {
                match (&message.call, &ringing) {
                    (Some(CallNote::Ringing { url }), Some(convert::Ringing::Incoming { id }))
                        if may_ring(app, shared, message) =>
                    {
                        // The dialog gets a name the push did not bring from the directory.
                        let caller = author.clone().unwrap_or_else(|| stand_in(message.author.clone()));
                        let ring = crate::ring::Ring {
                            id: id.clone(),
                            chat_id: message.chat_id.0.clone(),
                            url: url.clone(),
                            caller,
                        };
                        crate::ring::incoming(app, ring);
                    }
                    (Some(CallNote::Ended { url, .. }), _) => crate::ring::call_ended(app, url),
                    _ => {}
                }
            }
            // A message from a chat the list does not have (someone started a direct chat or
            // added the user to a group): fetch the chat, then show the message.
            if let LiveEvent::Upsert { message, .. } = &ev
                && message.thread_parent.is_none()
                && let Ok(mut sh) = shared.lock()
                && sh.ready
                && !sh.titles.contains_key(&message.chat_id.0)
            {
                let chat = message.chat_id.0.clone();
                let first = !sh.pending.contains_key(&chat);
                // A note about members brings no chat: it may say the user was just removed.
                if first && message.system.is_some() {
                    return;
                }
                sh.pending.entry(chat.clone()).or_default().push(ev);
                if first {
                    let (app, shared) = (app.clone(), shared.clone());
                    tauri::async_runtime::spawn(async move { add_chat(app, shared, chat).await });
                }
                return;
            }
            if let LiveEvent::Upsert { message, mentioned, .. } = &ev {
                learn_mentions(app, shared, message, mentioned);
            }
            if let LiveEvent::Upsert { message, author, .. } = &ev
                && !message.is_ringing()
                && reply_notifies_now(app, shared, message, author.as_ref())
            {
                notify(app, shared, message, author.as_ref());
            }
            let _ = app.emit(EVENT, &ev);
        }
        Incoming::Text(_) => {}
        Incoming::Closed => {
            let _ = app.emit(EVENT, &LiveEvent::Connection { online: false });
            let app = app.clone();
            tauri::async_runtime::spawn(async move { reconnect_loop(app, generation).await });
        }
    }
}

/// Applies a chat or settings change from a `StateSync` push to `Shared`. Returns whether the UI
/// gets the event: a new chat reaches it later, as `ChatAdded`.
fn sync_shared(app: &AppHandle, shared: &Arc<StdMutex<Shared>>, ev: &LiveEvent) -> bool {
    let Ok(mut sh) = shared.lock() else { return true };
    match ev {
        LiveEvent::Joined { chat_id } => {
            if sh.ready {
                let (app, shared, chat) = (app.clone(), shared.clone(), chat_id.0.clone());
                tauri::async_runtime::spawn(async move { join_later(app, shared, chat).await });
            }
            false
        }
        LiveEvent::ChatRemoved { chat_id } => {
            let id = &chat_id.0;
            sh.forget_title(id);
            sh.last.remove(id);
            sh.muted.remove(id);
            sh.pending.remove(id);
            true
        }
        LiveEvent::ChatChanged {
            chat_id,
            name: Some(name),
            ..
        } => {
            // Direct chats are titled by the person, not by the chat name.
            if chat_id.0.contains('/')
                && let Some(title) = sh.titles.get_mut(&chat_id.0)
            {
                let old = std::mem::replace(title, mostik_core::group_label(name));
                if let Some(c) = sh.channels.iter_mut().find(|c| mostik_core::group_label(c) == old) {
                    *c = name.clone();
                }
            }
            true
        }
        LiveEvent::Mutings { chats } => {
            sh.muted = chats.iter().map(|(id, level)| (id.0.clone(), *level)).collect();
            true
        }
        LiveEvent::Paused { pause } => {
            sh.paused = *pause;
            true
        }
        _ => true,
    }
}

/// A `StateSync` says the user is in a new chat. When the user created or joined it here, the
/// command that did it records the chat a moment later: only a chat still unknown is fetched.
async fn join_later(app: AppHandle, shared: Arc<StdMutex<Shared>>, chat_id: String) {
    tokio::time::sleep(Duration::from_secs(2)).await;
    {
        let Ok(mut sh) = shared.lock() else { return };
        if sh.titles.contains_key(&chat_id) || sh.pending.contains_key(&chat_id) {
            return;
        }
        sh.pending.insert(chat_id.clone(), vec![]);
    }
    add_chat(app, shared, chat_id).await;
}

/// Fetches a chat that is not in the list yet (`get_chats_info`, plus the profile of a direct
/// chat's peer the app does not know) and hands it to the UI together with the events held for it.
async fn add_chat(app: AppHandle, shared: Arc<StdMutex<Shared>>, chat_id: String) {
    let state = app.state::<AppState>();
    let mut fetched = None;
    // Right after a (re)connection the new `Live` may not be stored yet.
    for _ in 0..10 {
        match live(&state).await {
            Ok(live) if Arc::ptr_eq(&live.shared, &shared) => {
                fetched = fetch_chat(&live, &chat_id).await;
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    if let Some((chat, _)) = &fetched
        && service::is_service_chat(chat)
    {
        if let Ok(mut sh) = shared.lock() {
            sh.pending.remove(&chat_id);
        }
        service::adopt(&app, &shared, chat);
        return;
    }
    let held = {
        let Ok(mut sh) = shared.lock() else { return };
        let held = sh.pending.remove(&chat_id).unwrap_or_default();
        match fetched {
            Some((chat, users)) => {
                let title = match &chat.kind {
                    ChatKind::Direct { peer } => users
                        .iter()
                        .find(|u| &u.id == peer)
                        .map(|u| u.display_name.clone())
                        .or_else(|| sh.names.get(&peer.0).cloned())
                        .unwrap_or_default(),
                    _ => mostik_core::group_label(&chat.name),
                };
                sh.note_chat(&chat, title);
                sh.learn_names(&users);
                let section = chat.home_section().to_owned();
                let _ = app.emit(EVENT, &LiveEvent::ChatAdded { chat, section, users });
            }
            // Known from now on even if the fetch failed: its messages are not held again.
            None => {
                sh.titles.insert(chat_id.clone(), String::new());
            }
        }
        // Emitted under the lock: events that arrive meanwhile wait and come after these.
        for ev in &held {
            let _ = app.emit(EVENT, ev);
        }
        held
    };
    for ev in held {
        if let LiveEvent::Upsert {
            message,
            author,
            mentioned,
        } = &ev
        {
            learn_mentions(&app, &shared, message, mentioned);
            if reply_notifies_now(&app, &shared, message, author.as_ref()) {
                notify(&app, &shared, message, author.as_ref());
            }
        }
    }
}

/// People mentioned as `@<guid>` in `messages` who are not among `known`: stand-ins for the
/// directory to name.
pub(super) fn unnamed_mentions(messages: &[Message], known: &[User]) -> Vec<User> {
    let mut out: Vec<User> = Vec::new();
    for id in messages
        .iter()
        .flat_map(|m| mostik_core::markup::mentioned_ids(&m.text))
    {
        if !known.iter().chain(&out).any(|u| u.id.0 == id) {
            out.push(stand_in(UserId(id)));
        }
    }
    out
}

/// A person known only by id, until the directory names them.
fn stand_in(id: UserId) -> User {
    User::named(id, "")
}

/// Names of the people a new message mentions: the ones Telemost sent with it are recorded,
/// the rest are asked from the directory and handed to the UI as [`LiveEvent::Users`].
fn learn_mentions(app: &AppHandle, shared: &Arc<StdMutex<Shared>>, m: &Message, mentioned: &[User]) {
    let unknown: Vec<User> = {
        let Ok(mut sh) = shared.lock() else { return };
        sh.learn_names(mentioned);
        let mut unknown = unnamed_mentions(std::slice::from_ref(m), mentioned);
        unknown.retain(|u| !sh.names.contains_key(&u.id.0));
        unknown
    };
    if unknown.is_empty() {
        return;
    }
    let (app, shared) = (app.clone(), shared.clone());
    tauri::async_runtime::spawn(async move {
        let Ok(live) = live(&app.state::<AppState>()).await else {
            return;
        };
        let mut users = unknown;
        fill_profiles(&live.web, &mut users).await;
        users.retain(User::has_name);
        if users.is_empty() {
            return;
        }
        if let Ok(mut sh) = shared.lock() {
            sh.learn_names(&users);
        }
        let _ = app.emit(EVENT, &LiveEvent::Users { users });
    });
}

async fn fetch_chat(live: &Live, chat_id: &str) -> Option<(Chat, Vec<User>)> {
    let info = chat_info_raw(live, chat_id)
        .await
        .map_err(|e| eprintln!("new chat {chat_id}: {e}"))
        .ok()?;
    let chat = convert::chat_from_info(&info, Some(&live.me))?;
    let mut users = vec![];
    if let ChatKind::Direct { peer } = &chat.kind {
        let known = live.shared.lock().is_ok_and(|sh| sh.names.contains_key(&peer.0));
        if !known {
            let mut peer = vec![stand_in(peer.clone())];
            fill_profiles(&live.web, &mut peer).await;
            for u in &mut peer {
                if !u.has_name() {
                    u.display_name = mostik_core::UNKNOWN_USER.into();
                }
            }
            users.extend(peer);
        }
    }
    Some((chat, users))
}
