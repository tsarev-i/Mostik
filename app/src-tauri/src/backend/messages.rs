//! History pages and what is done to messages: sending, editing, deleting, reactions, read
//! markers, pins and forwarding.

use std::sync::Arc;
use std::time::Duration;

use mostik_core::{ChatId, Message, Timestamp};
use mostik_ipc::cmd::{
    self, Args, ChatArgs, EditArgs, ForwardArgs, HistoryArgs, ImportantArgs, MsgArgs, Out, PinArgs, ReactionArgs,
    SendArgs, UnreadArgs,
};
use mostik_ipc::{HistoryPage, Sent, SentText, Unsent};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::{id_pattern, shape};
use mostik_yandex::wire::{self, HistoryRequest, request_id};
use mostik_yandex::xiva_client::XivaError;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use super::events::unnamed_mentions;
use super::files::prefetch_previews;
use super::{AppState, EVENT, Live, fill_profiles, live, note_files, push, sending_options, target_chat};

/// History page sizes. A page of 50 takes the server 0.5–1.5 s in a busy channel and 20 about
/// 0.1–0.3 s, so a chat opens with a short page; older pages and threads take more at once.
const FIRST_PAGE: u32 = 20;
const OLDER_PAGE: u32 = 40;
const THREAD_PAGE: u32 = 50;
/// Seconds a history page may take: twice that with the repeat, about what one wait used to be.
const HISTORY_TIMEOUT_S: u64 = 12;

/// History page: the latest messages before `before` (exclusive). The page comes back at once;
/// directory profiles of its new authors follow as [`LiveEvent::Users`].
#[tauri::command]
pub async fn history(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    args: Args<cmd::History>,
) -> Result<Out<cmd::History>, String> {
    let HistoryArgs {
        chat_id: ChatId(chat_id),
        before,
        thread,
        limit,
    } = args;
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    let page = match (thread, before) {
        (Some(_), _) => THREAD_PAGE,
        (None, None) => FIRST_PAGE,
        (None, Some(_)) => OLDER_PAGE,
    };
    // A smaller page when asked, never a larger one.
    let limit = limit.map_or(page, |l| l.clamp(1, page));
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
    note_files(&state, &live.me, &data.messages);
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
pub async fn mark_read(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::MarkRead>,
) -> Result<Out<cmd::MarkRead>, String> {
    let ChatArgs {
        chat_id: ChatId(chat_id),
    } = args;
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

/// Sends a text message; with `reply`, a reply to a message of the same chat or thread. A text
/// longer than [`wire::MESSAGE_LIMIT`] goes as several messages, one after another, as the web
/// client sends it (see [`super::sending_options`]): the reply and the options go with the first.
/// When a later part does not go, the ones that went are returned with the rest.
#[tauri::command]
pub async fn send_message(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::SendMessage>,
) -> Result<Out<cmd::SendMessage>, String> {
    let SendArgs {
        chat_id: ChatId(chat_id),
        text,
        thread,
        reply,
        options,
    } = args;
    if mostik_core::split::utf16_len(&text) > wire::TEXT_MAX {
        return Err(cmd::MESSAGE_TOO_LONG.to_owned());
    }
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    let parts = split_text(&live, &text);
    let mut messages = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        let msg = match (&reply, i) {
            (Some(r), 0) => wire::reply(&target, &request_id(), part, &target, r.ts, r.quote.as_deref()),
            _ => wire::plain_text(&target, &request_id(), part),
        };
        let (notify, important) = sending_options(options, i == 0);
        let sent = match push(&live, wire::with_options(msg, notify, important))
            .await
            .and_then(|r| sent_from_reply(&r))
        {
            Ok(sent) => sent,
            // What has gone stays: the feed has it from the pushes.
            Err(why) if i > 0 => {
                let unsent = Unsent {
                    text: parts[i..].join("\n"),
                    why,
                };
                return Ok(SentText {
                    messages,
                    unsent: Some(unsent),
                });
            }
            Err(e) => return Err(e),
        };
        messages.push(Message {
            chat_id: ChatId(chat_id.clone()),
            ts: sent.ts,
            author: live.me.clone(),
            text: part.clone(),
            thread_parent: thread,
            edited: false,
            important,
            reactions: vec![],
            thread: None,
            seq_no: sent.seq_no,
            attachments: vec![],
            forwarded: vec![],
            call: None,
            system: None,
            media: None,
            reads: None,
        });
    }
    Ok(SentText { messages, unsent: None })
}

/// `text` in the parts it goes in (see [`mostik_core::split::split_long`]), keeping whole the
/// mentions of the people and chats known by name.
pub(super) fn split_text(live: &Live, text: &str) -> Vec<String> {
    let split = |who: &mostik_core::markup::Mentionable| mostik_core::split::split_long(text, wire::MESSAGE_LIMIT, who);
    match live.shared.lock() {
        Ok(sh) => super::notify::with_names(&sh, split),
        Err(_) => split(&mostik_core::markup::Mentionable::default()),
    }
}

/// Edits the text of a message. `whole`: the message has images, files or forwarded messages,
/// so the edit resends all of it with the new text; a text-only edit would drop them.
#[tauri::command]
pub async fn edit_message(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::EditMessage>,
) -> Result<Out<cmd::EditMessage>, String> {
    let EditArgs {
        chat_id: ChatId(chat_id),
        ts,
        text,
        thread,
        whole,
    } = args;
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    let msg = if whole {
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
    args: Args<cmd::DeleteMessage>,
) -> Result<Out<cmd::DeleteMessage>, String> {
    let MsgArgs {
        chat_id: ChatId(chat_id),
        ts,
        thread,
    } = args;
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    push(&live, wire::delete(&target, ts)).await.map(|_| ())
}

#[tauri::command]
pub async fn set_reaction(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::SetReaction>,
) -> Result<Out<cmd::SetReaction>, String> {
    let ReactionArgs {
        chat_id: ChatId(chat_id),
        ts,
        kind,
        add,
        thread,
    } = args;
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    push(&live, wire::reaction(&target, ts, kind, add)).await.map(|_| ())
}

#[tauri::command]
pub async fn set_important(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::SetImportant>,
) -> Result<Out<cmd::SetImportant>, String> {
    let ImportantArgs {
        chat_id: ChatId(chat_id),
        ts,
        important,
        thread,
    } = args;
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    push(&live, wire::importance(&target, ts, important)).await.map(|_| ())
}

#[tauri::command]
pub async fn typing(state: tauri::State<'_, AppState>, args: Args<cmd::Typing>) -> Result<Out<cmd::Typing>, String> {
    let ChatArgs {
        chat_id: ChatId(chat_id),
    } = args;
    let live = live(&state).await?;
    push(&live, wire::typing(&chat_id)).await.map(|_| ())
}

/// "Mark unread" from message `ts` (the message itself becomes unread).
#[tauri::command]
pub async fn mark_unread(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::MarkUnread>,
) -> Result<Out<cmd::MarkUnread>, String> {
    let UnreadArgs {
        chat_id: ChatId(chat_id),
        ts,
        seq_no,
    } = args;
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
    args: Args<cmd::PinMessage>,
) -> Result<Out<cmd::PinMessage>, String> {
    let PinArgs {
        chat_id: ChatId(chat_id),
        ts,
    } = args;
    let live = live(&state).await?;
    let mut pin = json!({ "ChatId": chat_id });
    if let Some(ts) = ts {
        pin["Timestamp"] = json!(ts);
    }
    push(&live, json!({ "Pin": pin })).await.map(|_| ())
}

/// [`Sent`] from Telemost's answer to the push of a new message.
pub(super) fn sent_from_reply(resp: &Value) -> Result<Sent, String> {
    Ok(Sent {
        ts: resp
            .pointer("/MessageInfo/TimestampMcs")
            .and_then(convert::uint)
            .ok_or("Telemost did not return the message time")?,
        seq_no: resp.pointer("/MessageInfo/SeqNo").and_then(convert::uint).unwrap_or(0),
    })
}

/// Forwards message `ts` from `from_chat` (a thread reply from the thread chat) to `to_chat` with a comment.
#[tauri::command]
pub async fn forward_message(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::ForwardMessage>,
) -> Result<Out<cmd::ForwardMessage>, String> {
    let ForwardArgs {
        from_chat: ChatId(from_chat),
        ts,
        thread,
        to_chat: ChatId(to_chat),
        text,
    } = args;
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
    sent_from_reply(&push(&live, json!({ "Plain": plain })).await?)
}
