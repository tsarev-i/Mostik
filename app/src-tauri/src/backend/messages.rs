//! History pages and what is done to messages: sending, editing, deleting, reactions, read
//! markers, pins and forwarding.

use std::sync::Arc;
use std::time::Duration;

use mostik_core::{ChatId, Message, Timestamp, User};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::{id_pattern, shape};
use mostik_yandex::wire::{self, HistoryRequest, request_id};
use mostik_yandex::xiva_client::XivaError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use super::events::unnamed_mentions;
use super::files::prefetch_previews;
use super::{AppState, EVENT, Live, fill_profiles, live, push, target_chat};

/// History page sizes. A page of 50 takes the server 0.5–1.5 s in a busy channel and 20 about
/// 0.1–0.3 s, so a chat opens with a short page; older pages and threads take more at once.
const FIRST_PAGE: u32 = 20;
const OLDER_PAGE: u32 = 40;
const THREAD_PAGE: u32 = 50;
/// Seconds a history page may take: twice that with the repeat, about what one wait used to be.
const HISTORY_TIMEOUT_S: u64 = 12;

/// A history page and the authors of its messages the app has not seen yet: the chat list at
/// start-up holds one message per chat, so most authors of an opened chat arrive only here.
#[derive(Serialize)]
pub struct HistoryPage {
    messages: Vec<Message>,
    users: Vec<User>,
    /// The server may have older messages. Decided by the raw page: service records such as
    /// joins are dropped by the parser, so fewer messages than asked does not mean the end.
    more: bool,
    /// The time of the oldest record of the page, deleted messages and service records included:
    /// the next page starts before it, or a run of deleted messages would be asked for again.
    oldest: Option<Timestamp>,
    /// How far the others have read the chat (`LastSeenTsMcs`); 0 for a thread.
    peer_seen: Timestamp,
}

/// History page: the latest messages before `before` (exclusive). The page comes back at once;
/// directory profiles of its new authors follow as [`LiveEvent::Users`].
#[tauri::command]
pub async fn history(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    chat_id: String,
    before: Option<Timestamp>,
    thread: Option<Timestamp>,
) -> Result<HistoryPage, String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    let limit = match (thread, before) {
        (Some(_), _) => THREAD_PAGE,
        (None, None) => FIRST_PAGE,
        (None, Some(_)) => OLDER_PAGE,
    };
    let started = std::time::Instant::now();
    let (live, body) = history_page(&state, live, &target, limit, before).await?;
    let raw = body
        .pointer("/Chats/0/Messages")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let data = convert::from_history(&body, Some(&live.me));
    let mut users = data.users;
    users.extend(unnamed_mentions(&data.messages, &users));
    if let Ok(mut sh) = live.shared.lock() {
        if before.is_none() && thread.is_none() {
            sh.last.extend(data.last);
        }
        // Someone already known by name needs no directory request; a stand-in name is not known.
        users.retain(|u| !sh.names.contains_key(&u.id.0));
        // Recorded right away, so the next page does not ask the directory for the same people.
        sh.learn_names(&users);
    }
    eprintln!(
        "history {}: {} of {limit} records, {} messages, {} new authors, {} ms",
        id_pattern(&chat_id),
        raw,
        data.messages.len(),
        users.len(),
        started.elapsed().as_millis()
    );
    if !users.is_empty() {
        let mut profiles = users.clone();
        let live = live.clone();
        tauri::async_runtime::spawn(async move {
            fill_profiles(&live.web, &mut profiles).await;
            if let Ok(mut sh) = live.shared.lock() {
                sh.learn_names(&profiles);
            }
            let _ = app.emit(EVENT, &LiveEvent::Users { users: profiles });
        });
    }
    prefetch_previews(live.clone(), &data.messages);
    let peer_seen = match thread {
        None => data.chats.first().map_or(0, |c| c.peer_seen_ts),
        Some(_) => 0,
    };
    Ok(HistoryPage {
        messages: data.messages,
        users,
        more: raw >= limit as usize,
        oldest: convert::oldest_record(&body),
        peer_seen,
    })
}

/// One history page. Reading is safe to repeat, so a slow answer is asked for once more, and
/// a connection that closed meanwhile is waited for and asked again. Returns the connection
/// that answered.
pub(super) async fn history_page(
    state: &AppState,
    mut live: Arc<Live>,
    target: &str,
    limit: u32,
    before: Option<Timestamp>,
) -> Result<(Arc<Live>, Value), String> {
    let mut retried = false;
    loop {
        let result = live
            .xiva
            .call(
                "history",
                &HistoryRequest::page(target, limit, before),
                HISTORY_TIMEOUT_S,
            )
            .await;
        let err = match result {
            Ok(body) if body.get("Chats").is_some() => return Ok((live, body)),
            // Not a page: an error answer must not read as the end of the history.
            Ok(body) => return Err(format!("Telemost did not return the history: {}", shape(&body))),
            Err(e) => e,
        };
        if retried || !matches!(err, XivaError::Timeout(..) | XivaError::Closed) {
            return Err(err.to_string());
        }
        eprintln!("history {}: {err}, asking again", id_pattern(target));
        retried = true;
        // After a close the reconnection brings a new connection; wait for it a little.
        for _ in 0..20 {
            match self::live(state).await {
                Ok(l) if !l.xiva.is_closed() => {
                    live = l;
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    }
}

#[tauri::command]
pub async fn mark_read(state: tauri::State<'_, AppState>, chat_id: String) -> Result<(), String> {
    let live = live(&state).await?;
    let Some((ts, seq)) = live.shared.lock().ok().and_then(|mut sh| {
        let last = sh.last.get(&chat_id).copied().filter(|(ts, seq)| *ts > 0 && *seq > 0)?;
        (sh.seen_sent.insert(chat_id.clone(), last) != Some(last)).then_some(last)
    }) else {
        return Ok(());
    };
    let sent = push(&live, wire::seen_marker(&chat_id, ts, seq)).await.map(|_| ());
    if sent.is_err()
        && let Ok(mut sh) = live.shared.lock()
        && sh.seen_sent.get(&chat_id) == Some(&(ts, seq))
    {
        sh.seen_sent.remove(&chat_id);
    }
    sent
}

/// What a message replies to: message `ts` of the chat (or thread) it is sent to, with the quoted fragment.
#[derive(Debug, Deserialize)]
pub struct ReplyRef {
    ts: Timestamp,
    quote: Option<String>,
}

/// Sends a text message; with `reply`, a reply to a message of the same chat or thread.
#[tauri::command]
pub async fn send_message(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    text: String,
    thread: Option<Timestamp>,
    reply: Option<ReplyRef>,
) -> Result<Message, String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    let msg = match &reply {
        Some(r) => wire::reply(&target, &request_id(), &text, &target, r.ts, r.quote.as_deref()),
        None => wire::plain_text(&target, &request_id(), &text),
    };
    let sent = Sent::from_reply(&push(&live, msg).await?)?;
    Ok(Message {
        chat_id: ChatId(chat_id),
        ts: sent.ts,
        author: live.me.clone(),
        text,
        thread_parent: thread,
        edited: false,
        important: false,
        reactions: vec![],
        thread: None,
        seq_no: sent.seq_no,
        attachments: vec![],
        forwarded: vec![],
        call: None,
        system: None,
        reads: None,
    })
}

/// Edits the text of a message. `whole`: the message has images, files or forwarded messages,
/// so the edit resends all of it with the new text; a text-only edit would drop them.
#[tauri::command]
pub async fn edit_message(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Timestamp,
    text: String,
    thread: Option<Timestamp>,
    whole: Option<bool>,
) -> Result<(), String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    let msg = if whole.unwrap_or(false) {
        let original = original_plain(&live, &target, ts).await?;
        wire::edit_whole(&original, &target, ts, &text)
    } else {
        wire::edit_text(&target, ts, &request_id(), &text)
    };
    push(&live, msg).await.map(|_| ())
}

/// The `Plain` of message `ts` in chat `target` as Telemost keeps it, like the web client's
/// `requestMessage`: the one message before `ts + 1`.
pub(super) async fn original_plain(live: &Live, target: &str, ts: Timestamp) -> Result<Value, String> {
    let page = live
        .xiva
        .call(
            "history",
            &HistoryRequest::page(target, 1, Some(ts + 1)),
            HISTORY_TIMEOUT_S,
        )
        .await
        .map_err(|e| e.to_string())?;
    page.pointer("/Chats/0/Messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|m| m.get("ServerMessage").unwrap_or(m))
        .find(|m| m.pointer("/ServerMessageInfo/Timestamp").and_then(Value::as_u64) == Some(ts))
        .and_then(|m| m.pointer("/ClientMessage/Plain"))
        .cloned()
        .ok_or_else(|| "Telemost did not return the message being edited".to_owned())
}

#[tauri::command]
pub async fn delete_message(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Timestamp,
    thread: Option<Timestamp>,
) -> Result<(), String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    push(&live, wire::delete(&target, ts)).await.map(|_| ())
}

#[tauri::command]
pub async fn set_reaction(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Timestamp,
    kind: u64,
    add: bool,
    thread: Option<Timestamp>,
) -> Result<(), String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    push(&live, wire::reaction(&target, ts, kind, add)).await.map(|_| ())
}

#[tauri::command]
pub async fn set_important(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Timestamp,
    important: bool,
    thread: Option<Timestamp>,
) -> Result<(), String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    push(&live, wire::importance(&target, ts, important)).await.map(|_| ())
}

#[tauri::command]
pub async fn typing(state: tauri::State<'_, AppState>, chat_id: String) -> Result<(), String> {
    let live = live(&state).await?;
    push(&live, wire::typing(&chat_id)).await.map(|_| ())
}

/// "Mark unread" from message `ts` (the message itself becomes unread).
#[tauri::command]
pub async fn mark_unread(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Timestamp,
    seq_no: u64,
) -> Result<(), String> {
    let live = live(&state).await?;
    // Reading the chat again must send the marker again.
    if let Ok(mut sh) = live.shared.lock() {
        sh.seen_sent.remove(&chat_id);
    }
    push(
        &live,
        json!({ "UnseenMarker": { "ChatId": chat_id, "Timestamp": ts, "SeqNo": seq_no } }),
    )
    .await
    .map(|_| ())
}

/// Pins a message in the chat (`ts`) or unpins it (`None`).
#[tauri::command]
pub async fn pin_message(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Option<Timestamp>,
) -> Result<(), String> {
    let live = live(&state).await?;
    let mut pin = json!({ "ChatId": chat_id });
    if let Some(ts) = ts {
        pin["Timestamp"] = json!(ts);
    }
    push(&live, json!({ "Pin": pin })).await.map(|_| ())
}

/// A sent message: time and number.
#[derive(Serialize)]
pub struct Sent {
    pub(super) ts: Timestamp,
    pub(super) seq_no: u64,
}

impl Sent {
    /// From Telemost's answer to the push of a new message.
    pub(super) fn from_reply(resp: &Value) -> Result<Self, String> {
        Ok(Self {
            ts: resp
                .pointer("/MessageInfo/TimestampMcs")
                .and_then(convert::uint)
                .ok_or("Telemost did not return the message time")?,
            seq_no: resp.pointer("/MessageInfo/SeqNo").and_then(convert::uint).unwrap_or(0),
        })
    }
}

/// Forwards message `ts` from `from_chat` (a thread reply from the thread chat) to `to_chat` with a comment.
#[tauri::command]
pub async fn forward_message(
    state: tauri::State<'_, AppState>,
    from_chat: String,
    ts: Timestamp,
    thread: Option<Timestamp>,
    to_chat: String,
    text: String,
) -> Result<Sent, String> {
    let live = live(&state).await?;
    let source = target_chat(&from_chat, thread)?;
    let mut plain = json!({
        "ChatId": to_chat,
        "PayloadId": request_id(),
        "ForwardedMessageRefs": [{ "ChatId": source, "Timestamp": ts }],
    });
    if !text.trim().is_empty() {
        plain["Text"] = json!({ "MessageText": text.trim() });
    }
    Sent::from_reply(&push(&live, json!({ "Plain": plain })).await?)
}
