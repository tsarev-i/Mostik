//! Calls from a chat, like Slack huddles: starting or joining one, and the call window from the
//! sidebar.

use mostik_yandex::convert;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use super::{AppState, live, rpc};
use crate::call;

/// A call from a chat, like a Slack huddle: the meeting bound to the chat (`create_meeting`) opens
/// in the call window (`call.rs`). `meeting_id` is the meeting link itself. `label` is what the
/// window title and the sidebar show: `#dev` or the person's name. `url` is the link of a call
/// already going on in the chat: joining it needs no `create_meeting`.
#[tauri::command]
pub async fn start_call(app: AppHandle, chat_id: String, label: String, url: Option<String>) -> Result<(), String> {
    open_call(&app, chat_id, label, url).await
}

/// See [`start_call`]. Joining a call that rings for the user answers it.
pub(crate) async fn open_call(
    app: &AppHandle,
    chat_id: String,
    label: String,
    url: Option<String>,
) -> Result<(), String> {
    if let Some(url) = &url {
        crate::ring::accept(app, url);
    }
    let state = app.state::<AppState>();
    let info = call::CallInfo {
        chat_id: Some(chat_id.clone()),
        label,
    };
    // Runs only when no call is open: `create_meeting` announces the call in the chat. An open
    // call comes to the front even without a connection.
    let prepare = || async {
        let live = live(&state).await?;
        if let Some(url) = url {
            return Ok((url, live.web.jar().clone()));
        }
        let v = rpc(
            &live,
            "create_meeting",
            json!({ "chat_id": chat_id, "supported_features": [] }),
        )
        .await?;
        let id = v
            .pointer("/data/meeting_id")
            .and_then(Value::as_str)
            .ok_or("Telemost did not return the meeting link")?;
        Ok((convert::meeting_url(id), live.web.jar().clone()))
    };
    call::open_chat_call(app, info, prepare).await
}

/// The call window comes to the front (where the desktop lets an app raise its windows:
/// Windows and X11 do, Wayland compositors usually do not).
#[tauri::command]
pub async fn focus_call(app: AppHandle) {
    call::focus(&app).await;
}

/// "Leave" in the sidebar call bar: the call window closes.
#[tauri::command]
pub fn leave_call(app: AppHandle) {
    call::close(&app);
}

/// The current call, for a UI that has just loaded.
#[tauri::command]
pub fn current_call(app: AppHandle) -> Option<call::CallInfo> {
    call::info(&app)
}
