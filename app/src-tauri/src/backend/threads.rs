//! Followed threads: the list for "Threads", and which thread replies notify.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use mostik_core::event::Alert;
use mostik_core::{ChatId, Message, Timestamp, User};
use mostik_ipc::Threads;
use mostik_ipc::cmd::{self, Args, Out, ThreadReadArgs};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::shape;
use mostik_yandex::wire;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use super::connection::{id_patterns, probing, write_probe};
use super::notify::{mentions_me, notify};
use super::{AppState, EVENT, Live, Shared, live, note_files, push, target_chat};

/// Threads the user follows, with unread counts, like Slack's "Threads". The UI asks once per
/// connection and then keeps the list current from the pushes. The thread ID format goes to
/// the diagnostics.
#[tauri::command]
pub async fn followed_threads(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Out<cmd::FollowedThreads>, String> {
    let live = live(&state).await?;
    let v = live.xiva.call("history", &wire::thread_list(0), 30).await;
    if probing()
        && let Ok(v) = &v
    {
        write_probe(&app, "probe-threads.json", &{
            let mut entry = id_patterns(v);
            entry["shape"] = shape(v);
            entry
        });
    }
    let (threads, users) = remember_threads(&state, &live, &v.map_err(|e| e.to_string())?);
    Ok(Threads { threads, users })
}

/// Parses the followed threads and records them as the ones whose replies notify, and their
/// attachments as ones the UI may save. Threads the user left meanwhile stay out.
fn remember_threads(state: &AppState, live: &Live, v: &Value) -> (Vec<mostik_core::FollowedThread>, Vec<User>) {
    let (mut threads, users) = convert::followed_threads(v, Some(&live.me));
    if let Ok(mut sh) = live.shared.lock() {
        threads.retain(|t| !sh.service.contains(&t.parent.chat_id.0));
        threads.retain(|t| !sh.left.contains(&(t.parent.chat_id.0.clone(), t.parent.ts)));
        sh.followed = threads
            .iter()
            .map(|t| (t.parent.chat_id.0.clone(), t.parent.ts))
            .collect();
        // Joined by a push: the list takes over those it has; the others it may not have yet.
        let listed = std::mem::take(&mut sh.followed);
        sh.joined.retain(|k| !listed.contains(k));
        sh.followed = listed;
    }
    note_files(state, &live.me, threads.iter().map(|t| &t.parent));
    (threads, users)
}

/// Fetches the followed threads again a moment after a reply in a thread the list did not
/// have (several replies wait for one fetch): Telemost may have subscribed the user to it,
/// for example for a mention. The UI gets the new list, and the waiting replies notify if
/// their thread is followed now.
pub(super) async fn refresh_threads(app: AppHandle, shared: Arc<StdMutex<Shared>>) {
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let fetched = match live(&app.state::<AppState>()).await {
        Ok(live) if Arc::ptr_eq(&live.shared, &shared) => live
            .xiva
            .call("history", &wire::thread_list(0), 30)
            .await
            .map(|v| remember_threads(&app.state::<AppState>(), &live, &v))
            .map_err(|e| eprintln!("threads: {e}"))
            .ok(),
        _ => None,
    };
    let waiting = {
        let Ok(mut sh) = shared.lock() else { return };
        let waiting = std::mem::take(&mut sh.thread_wait);
        if fetched.is_some() {
            for (m, ..) in &waiting {
                if let Some(key) = thread_key(m)
                    && !sh.follows(&key)
                {
                    sh.not_followed.insert(key);
                }
            }
        }
        waiting
    };
    let Some((threads, users)) = fetched else {
        // Without the list only the threads Telemost said the user joined are known to notify.
        for (m, author, alert) in waiting {
            if shared
                .lock()
                .is_ok_and(|sh| thread_key(&m).is_some_and(|k| sh.follows(&k)))
            {
                notify(&app, &shared, &m, author.as_ref(), alert);
            }
        }
        return;
    };
    eprintln!(
        "threads fetched again for {} replies: {} followed",
        waiting.len(),
        threads.len()
    );
    // Checked again and sent under the lock: a departure handled meanwhile either keeps its
    // thread out of this list or reaches the UI after it.
    if let Ok(sh) = shared.lock() {
        let threads: Vec<_> = threads
            .into_iter()
            .filter(|t| !sh.left.contains(&(t.parent.chat_id.0.clone(), t.parent.ts)))
            .collect();
        let _ = app.emit(EVENT, &LiveEvent::Threads { threads, users });
    }
    for (m, author, alert) in waiting {
        let followed = shared
            .lock()
            .is_ok_and(|sh| thread_key(&m).is_some_and(|k| sh.follows(&k)));
        if followed {
            notify(&app, &shared, &m, author.as_ref(), alert);
        }
    }
}

/// `(chat, parent)` of a thread reply.
fn thread_key(m: &Message) -> Option<(String, Timestamp)> {
    m.thread_parent.map(|p| (m.chat_id.0.clone(), p))
}

/// Whether a reply can notify now: it is in a followed thread or is not a reply. A reply in a
/// thread of unknown standing waits for the list to be fetched again (see [`refresh_threads`]).
pub(super) fn reply_notifies_now(
    app: &AppHandle,
    shared: &Arc<StdMutex<Shared>>,
    m: &Message,
    author: Option<&User>,
    alert: Alert,
) -> bool {
    let Some(key) = thread_key(m) else { return true };
    let Ok(mut sh) = shared.lock() else { return false };
    if sh.me.as_ref() == Some(&m.author) {
        // An edit is no new reply: it subscribes no one.
        if m.edited {
            return false;
        }
        // Telemost subscribes the user to a thread they reply in.
        sh.not_followed.remove(&key);
        let revived = sh.left.remove(&key);
        sh.followed.insert(key);
        // The UI dropped a thread the user left and cannot bring it back without its parent:
        // it gets the list fetched again.
        if revived && sh.ready && sh.thread_wait.is_empty() {
            let (app, shared) = (app.clone(), shared.clone());
            tauri::async_runtime::spawn(async move { refresh_threads(app, shared).await });
        }
        return false;
    }
    if sh.follows(&key) {
        return true;
    }
    if !m.edited && mentions_me(&sh, m) {
        // Telemost subscribes the user to a thread they are mentioned in, left or not.
        sh.left.remove(&key);
    }
    if m.edited || !sh.ready || (sh.not_followed.contains(&key) && !mentions_me(&sh, m)) {
        return false;
    }
    let first = sh.thread_wait.is_empty();
    sh.thread_wait.push((m.clone(), author.cloned(), alert));
    if first {
        let (app, shared) = (app.clone(), shared.clone());
        tauri::async_runtime::spawn(async move { refresh_threads(app, shared).await });
    }
    false
}

/// Marks the thread of message `parent` read up to its reply `ts` with number `seq_no`
/// (the thread chat's own numbering).
#[tauri::command]
pub async fn mark_thread_read(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::MarkThreadRead>,
) -> Result<Out<cmd::MarkThreadRead>, String> {
    let ThreadReadArgs {
        chat_id: ChatId(chat_id),
        parent,
        ts,
        seq_no,
    } = args;
    let live = live(&state).await?;
    let thread = target_chat(&chat_id, Some(parent))?;
    push(&live, wire::seen_marker(&thread, ts, seq_no)).await.map(|_| ())
}
