//! Presence: people's availability and statuses, the user's own status, and the heartbeat that
//! keeps the user online.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mostik_core::UserId;
use mostik_yandex::convert;
use mostik_yandex::wire::request_id;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use super::{AppState, Live, live, push};
use crate::call;

async fn subscribe_body(live: &Live, extra: Value) -> Result<Value, String> {
    let sub = live
        .xiva
        .wait_subscription(Duration::from_secs(5))
        .await
        .ok_or("no Xiva subscription")?;
    let mut body = json!({
        "RequestId": request_id(),
        "ClientTransportId": { "XivaSubscriptionId": sub },
        "UserAgent": live.user_agent,
        "TtlMcs": 600_000_000u64,
    });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    Ok(body)
}

/// Presence and statuses of people. The subscription also enables pushes of their changes for 10 minutes.
#[tauri::command]
pub async fn presence(
    state: tauri::State<'_, AppState>,
    guids: Vec<String>,
) -> Result<Vec<mostik_core::PresenceInfo>, String> {
    let live = live(&state).await?;
    let guids: Vec<String> = guids.into_iter().take(150).collect();
    let mut out: BTreeMap<String, mostik_core::PresenceInfo> = BTreeMap::new();
    for chunk in guids.chunks(50) {
        let body = subscribe_body(&live, json!({ "ToGuids": chunk, "MessageBodyTypes": [26] })).await?;
        if let Ok(v) = live.xiva.call("subscribe", &body, 20).await {
            for p in convert::statuses_from_subscribe(&v) {
                out.insert(p.user.0.clone(), p);
            }
        }
    }
    for chunk in guids.chunks(10) {
        let jobs = chunk.iter().map(|g| {
            let live = live.clone();
            let g = g.clone();
            async move {
                let body = subscribe_body(&live, json!({ "ToGuid": g, "MessageBodyType": 2 }))
                    .await
                    .ok()?;
                let v = live.xiva.call("subscribe", &body, 20).await.ok()?;
                Some((g, convert::online_from_heartbeat(&v)))
            }
        });
        for (g, online) in futures_util::future::join_all(jobs).await.into_iter().flatten() {
            out.entry(g.clone())
                .or_insert(mostik_core::PresenceInfo {
                    user: UserId(g),
                    online: None,
                    availability: Default::default(),
                    status: None,
                })
                .online = online;
        }
    }
    Ok(out.into_values().collect())
}

/// Own availability and status: `availability` 0–3, emoji and text (empty means no status),
/// `duration` is the number of seconds after which Telemost resets the status itself.
#[tauri::command]
pub async fn set_status(
    state: tauri::State<'_, AppState>,
    availability: u64,
    emoji: String,
    text: String,
    duration: Option<u64>,
) -> Result<(), String> {
    let live = live(&state).await?;
    // As the official web client does: "Do not disturb" also turns notifications off
    // (`NotificationMode` 1), on every device of the user.
    let dnd = availability == mostik_core::Availability::DoNotDisturb.code();
    let mut status = json!({ "Availability": availability, "NotificationMode": u64::from(dnd) });
    if !emoji.trim().is_empty() || !text.trim().is_empty() {
        status["CustomStatus"] = json!({ "Emoji": emoji.trim(), "Text": text.trim() });
    }
    if let Some(d) = duration.filter(|d| *d > 0) {
        status["Duration"] = json!(d);
    }
    // Telemost answers with the `status` bucket, which says whether notifications are paused.
    push(&live, json!({ "UserStatus": status })).await?;
    Ok(())
}

/// After a call: if Telemost still shows "In meeting" a few seconds after the window closed (the
/// page had no chance to say goodbye), take that status off. A status the user set stays.
pub fn clear_meeting_status_later(app: AppHandle) {
    // A call opened in the meantime, in the window or in the browser, has its own status.
    let opens = call::opens(&app);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(8)).await;
        if call::opens(&app) != opens || call::info(&app).is_some() {
            return;
        }
        let state = app.state::<AppState>();
        let Ok(live) = live(&state).await else { return };
        let me = std::slice::from_ref(&live.me.0);
        let Ok(body) = subscribe_body(&live, json!({ "ToGuids": me, "MessageBodyTypes": [26] })).await else {
            return;
        };
        let Ok(v) = live.xiva.call("subscribe", &body, 20).await else {
            return;
        };
        let Some(status) = convert::own_user_status(&v, &live.me).filter(|s| convert::is_meeting_status(s)) else {
            return;
        };
        let availability = status.get("Availability").and_then(Value::as_u64).unwrap_or(0);
        match push(&live, json!({ "UserStatus": { "Availability": availability } })).await {
            Ok(_) => eprintln!("call: cleared the meeting status left after the call"),
            Err(e) => eprintln!("call: could not clear the meeting status: {e}"),
        }
    });
}

/// Tells Telemost once a minute that you are online: "active" if the window is focused, otherwise "background".
pub(super) fn start_heartbeat(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.heartbeat_running.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<AppState>();
            if let Ok(live) = live(&state).await {
                let focused = app
                    .get_webview_window("main")
                    .and_then(|w| w.is_focused().ok())
                    .unwrap_or(false);
                let _ = push(&live, json!({ "Heartbeat": { "Type": if focused { 2 } else { 1 } } })).await;
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}
