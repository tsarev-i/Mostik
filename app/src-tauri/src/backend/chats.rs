//! Chats and people: profiles, direct chats, search, channels and their members, the synced chat
//! settings (notifications, folders, stars), browsing and joining public channels.

use mostik_core::{Chat, ChatFolder, ChatId, ChatKind, Message, NotifyLevel, User, UserId};
use mostik_yandex::clock;
use mostik_yandex::convert;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::AppHandle;

use super::{AppState, Live, live, rpc, rpc_ok, service, web_error};

/// A person's profile for the "Profile" panel.
#[tauri::command]
pub async fn user_profile(state: tauri::State<'_, AppState>, guid: String) -> Result<mostik_core::UserProfile, String> {
    let live = live(&state).await?;
    let mut web = live.web.clone_light();
    let v = web
        .rpc("get_users_data", json!({ "guids": [guid] }))
        .await
        .map_err(|e| e.to_string())?;
    convert::profiles(&v)
        .into_iter()
        .next()
        .ok_or_else(|| "profile not found".to_owned())
}

/// Direct chat with a person: existing or new (`create_private_chat` is idempotent).
#[tauri::command]
pub async fn open_dm(state: tauri::State<'_, AppState>, guid: String) -> Result<Chat, String> {
    let live = live(&state).await?;
    let mut web = live.web.clone_light();
    let v = web
        .rpc("create_private_chat", json!({ "guid": guid }))
        .await
        .map_err(|e| e.to_string())?;
    let id = v
        .pointer("/data/chat/chat_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Telemost did not open the chat: {:?}", v.pointer("/data/code")))?;
    let chat = Chat {
        member_count: 2,
        ..Chat::new(ChatId(id.to_owned()), convert::chat_kind(id, Some(&live.me), false))
    };
    note_chat(&live, &chat);
    Ok(chat)
}

/// Group details from `get_chats_info` (`{chat_ids: [...]}`).
pub(super) async fn chat_info_raw(live: &Live, chat_id: &str) -> Result<Value, String> {
    let v = rpc(live, "get_chats_info", json!({ "chat_ids": [chat_id] })).await?;
    v.pointer("/data/chats/0")
        .cloned()
        .ok_or_else(|| "Telemost did not return the chat details".to_owned())
}

/// Current chat version: Telemost rejects changes without it.
async fn chat_version(live: &Live, chat_id: &str) -> Result<u64, String> {
    chat_info_raw(live, chat_id)
        .await?
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "could not get the chat version".to_owned())
}

#[derive(Serialize)]
pub struct ChatDetails {
    name: String,
    description: String,
    role: String,
    member_count: u64,
}

impl ChatDetails {
    /// From a `get_chats_info` item.
    fn from_info(info: &Value) -> Self {
        let text = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        Self {
            name: text("name"),
            description: text("description"),
            role: text("role"),
            member_count: info.get("members_count").and_then(convert::uint).unwrap_or(0),
        }
    }
}

#[tauri::command]
pub async fn chat_details(state: tauri::State<'_, AppState>, chat_id: String) -> Result<ChatDetails, String> {
    let live = live(&state).await?;
    Ok(ChatDetails::from_info(&chat_info_raw(&live, &chat_id).await?))
}

/// People search for invitations. Without `chat_id` the search is relative to the notes chat.
#[tauri::command]
pub async fn search_people(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    query: String,
    chat_id: Option<String>,
) -> Result<Vec<mostik_core::UserProfile>, String> {
    let live = live(&state).await?;
    let chat = chat_id.unwrap_or_else(|| convert::notes_chat_id(&live.me).0);
    let mut web = live.web.clone_light();
    let v = web
        .rpc(
            "get_members_and_contacts_suggest",
            json!({ "query": query, "chat_id": chat, "limit": 20 }),
        )
        .await
        .map_err(|e| web_error(&app, e))?;
    rpc_ok(&v)?;
    Ok(convert::profiles(&v))
}

/// Server-side message search result.
#[derive(Serialize)]
pub struct FoundMessages {
    messages: Vec<Message>,
    users: Vec<User>,
}

/// Message search (HTTP RPC `search`). The server searches only within one chat (`chat_id`),
/// so the given chats are queried in parallel, at most 20.
#[tauri::command]
pub async fn search_messages(
    state: tauri::State<'_, AppState>,
    query: String,
    chat_ids: Vec<String>,
) -> Result<FoundMessages, String> {
    let live = live(&state).await?;
    let jobs = chat_ids.into_iter().take(20).map(|chat| {
        let mut web = live.web.clone_light();
        let query = query.clone();
        async move {
            web.rpc(
                "search",
                json!({ "query": query, "limit": 10, "entities": ["messages"], "chat_id": chat }),
            )
            .await
            .ok()
            .filter(|v| rpc_ok(v).is_ok())
        }
    });
    let (mut messages, mut users) = (Vec::new(), Vec::new());
    for v in futures_util::future::join_all(jobs).await.into_iter().flatten() {
        let (m, u) = convert::search_messages(&v, Some(&live.me));
        messages.extend(m);
        for x in u {
            if !users.iter().any(|y: &User| y.id == x.id) {
                users.push(x);
            }
        }
    }
    Ok(FoundMessages { messages, users })
}

/// New group (a channel in Slack terms). You are a member and the admin.
#[tauri::command]
pub async fn create_channel(
    state: tauri::State<'_, AppState>,
    name: String,
    description: String,
    members: Vec<String>,
    public: bool,
) -> Result<Chat, String> {
    let live = live(&state).await?;
    let mut all = vec![live.me.0.clone()];
    all.extend(members.into_iter().filter(|m| m != &live.me.0));
    let v = rpc(
        &live,
        "create_chat",
        json!({ "name": name, "members": all, "channel": false, "public": public }),
    )
    .await?;
    let id = v
        .pointer("/data/chat_id")
        .and_then(Value::as_str)
        .ok_or("Telemost did not return the chat ID")?
        .to_owned();
    // Known at once: the `StateSync` about the new chat does not fetch it again.
    if let Ok(mut sh) = live.shared.lock() {
        sh.titles
            .entry(id.clone())
            .or_insert_with(|| mostik_core::group_label(&name));
    }
    // Description and visibility go through `set_chat_info`, as in the web client (field `public`).
    if !description.trim().is_empty() || public {
        let version = v.pointer("/data/version").and_then(Value::as_u64).unwrap_or(0);
        rpc(
            &live,
            "set_chat_info",
            json!({ "chat_id": id, "version": version, "name": name, "description": description, "public": public }),
        )
        .await?;
    }
    // Name and description as the server saved them.
    let info = chat_info_raw(&live, &id).await.unwrap_or(Value::Null);
    let server_name = info
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or(name);
    let server_desc = info
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or(&description)
        .to_owned();
    let server_public = info.get("public").and_then(Value::as_bool).unwrap_or(false);
    if public && !server_public {
        eprintln!("channel created, but Telemost did not make it public");
    }
    let chat = Chat {
        name: server_name,
        description: (!server_desc.trim().is_empty()).then_some(server_desc),
        member_count: info
            .get("members_count")
            .and_then(Value::as_u64)
            .map_or(all.len() as u32, |n| n as u32),
        last_ts: clock::now_mcs(),
        ..Chat::new(ChatId(id), ChatKind::Group { public: server_public })
    };
    note_chat(&live, &chat);
    Ok(chat)
}

#[tauri::command]
pub async fn chat_members(
    state: tauri::State<'_, AppState>,
    chat_id: String,
) -> Result<Vec<mostik_core::ChatMember>, String> {
    let live = live(&state).await?;
    let v = rpc(&live, "get_chat_members", json!({ "chat_id": chat_id, "limit": 200 })).await?;
    Ok(convert::members(&v))
}

/// Adds (`add = true`) or removes members.
#[tauri::command]
pub async fn change_members(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    guids: Vec<String>,
    add: bool,
) -> Result<(), String> {
    let live = live(&state).await?;
    let version = chat_version(&live, &chat_id).await?;
    let users = if add {
        json!({ "add": guids })
    } else {
        json!({ "remove": guids })
    };
    rpc(
        &live,
        "change_chat_members",
        json!({ "chat_id": chat_id, "version": version, "users": users, "role": "member" }),
    )
    .await
    .map(drop)
}

#[tauri::command]
pub async fn update_chat(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    name: String,
    description: String,
) -> Result<ChatDetails, String> {
    let live = live(&state).await?;
    // Pass the visibility as is: without the `public` field the server may make a public channel private.
    let current = chat_info_raw(&live, &chat_id).await?;
    let version = current
        .get("version")
        .and_then(Value::as_u64)
        .ok_or("could not get the chat version")?;
    let public = current.get("public").and_then(Value::as_bool).unwrap_or(false);
    rpc(
        &live,
        "set_chat_info",
        json!({ "chat_id": chat_id, "version": version, "name": name, "description": description, "public": public }),
    )
    .await?;
    // Return what the server actually saved: that is what the UI shows.
    Ok(ChatDetails::from_info(&chat_info_raw(&live, &chat_id).await?))
}

#[tauri::command]
pub async fn leave_chat(state: tauri::State<'_, AppState>, chat_id: String) -> Result<(), String> {
    let live = live(&state).await?;
    let version = chat_version(&live, &chat_id).await?;
    rpc(&live, "leave", json!({ "chat_id": chat_id, "version": version })).await?;
    if let Ok(mut sh) = live.shared.lock() {
        sh.forget_title(&chat_id);
    }
    Ok(())
}

/// A chat joined or created after the start (see [`Shared::note_chat`](super::Shared::note_chat)).
fn note_chat(live: &Live, chat: &Chat) {
    if let Ok(mut sh) = live.shared.lock() {
        sh.note_chat(chat, mostik_core::group_label(&chat.name));
    }
}

/// Changes a synced setting (`set_bucket`): reads the current value and version,
/// applies `change` and writes it back whole. Retries on a version conflict.
async fn update_bucket(live: &Live, name: &str, change: impl Fn(&mut Value)) -> Result<Value, String> {
    let mut web = live.web.clone_light();
    for _ in 0..3 {
        let v = web.buckets().await.map_err(|e| e.to_string())?;
        rpc_ok(&v)?;
        let bucket = convert::bucket(&v, name);
        let mut value = bucket
            .and_then(|b| b.get("bucket_value"))
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let version = bucket
            .and_then(|b| b.get("version"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        change(&mut value);
        let r = web
            .rpc(
                "set_bucket",
                json!({ "bucket_name": name, "bucket_value": value, "version": version }),
            )
            .await
            .map_err(|e| e.to_string())?;
        if r.pointer("/data/code").and_then(Value::as_str) == Some("object_version_is_mismatch") {
            continue;
        }
        rpc_ok(&r)?;
        return Ok(value);
    }
    Err("Telemost did not accept the setting: it is being changed on another device at the same time".into())
}

/// What a chat notifies about (the `chat_mutings` bucket).
#[tauri::command]
pub async fn set_notify_level(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    level: NotifyLevel,
) -> Result<(), String> {
    let live = live(&state).await?;
    put_notify_level(&live, &chat_id, level).await?;
    if let Ok(mut sh) = live.shared.lock() {
        if level == NotifyLevel::All {
            sh.muted.remove(&chat_id);
        } else {
            sh.muted.insert(chat_id, level);
        }
    }
    Ok(())
}

/// Writes what chat `chat_id` notifies about into the `chat_mutings` bucket.
pub(super) async fn put_notify_level(live: &Live, chat_id: &str, level: NotifyLevel) -> Result<(), String> {
    let id = chat_id.to_owned();
    let (mute, mute_mentions) = level.flags();
    update_bucket(live, "chat_mutings", move |v| {
        v[&id] = json!({ "mute": mute, "mute_mentions": mute_mentions })
    })
    .await
    .map(drop)
}

/// Writes the organization's chat folders (the sidebar sections) into Telemost.
/// Returns the folders of every organization as they now are.
#[tauri::command]
pub async fn save_chat_folders(
    state: tauri::State<'_, AppState>,
    org: String,
    folders: Vec<ChatFolder>,
) -> Result<Vec<ChatFolder>, String> {
    let live = live(&state).await?;
    let value = update_bucket(&live, "folders", move |v| convert::put_chat_folders(v, &org, &folders)).await?;
    Ok(convert::chat_folders(&value))
}

/// "Star" / "Unstar" (the `pinned_chats` bucket). Returns the new list.
#[tauri::command]
pub async fn set_starred(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    starred: bool,
) -> Result<Vec<String>, String> {
    let live = live(&state).await?;
    let value = update_bucket(&live, "pinned_chats", move |v| {
        let mut list: Vec<Value> = v
            .get("pinned_chats")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.retain(|x| x.as_str() != Some(chat_id.as_str()));
        if starred {
            list.push(json!(chat_id));
        }
        v["pinned_chats"] = json!(list);
    })
    .await?;
    Ok(value["pinned_chats"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

/// Group from an HTTP RPC response (`get_recommended_chats`, `activate_chat`).
fn rpc_chat(v: &Value, me: &UserId) -> Option<Chat> {
    let id = v.get("chat_id")?.as_str()?;
    let public = v.get("public").and_then(Value::as_bool).unwrap_or(false)
        || v.get("invite_hash")
            .and_then(Value::as_str)
            .is_some_and(|h| !h.is_empty());
    Some(Chat {
        name: v.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
        description: v
            .get("description")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
            .map(str::to_owned),
        member_count: v.get("members_count").and_then(Value::as_u64).unwrap_or(0) as u32,
        organizations: v
            .get("organization_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(convert::uint)
            .collect(),
        ..Chat::new(ChatId(id.to_owned()), convert::chat_kind(id, Some(me), public))
    })
}

/// "Browse channels": public groups recommended by Telemost.
#[tauri::command]
pub async fn recommended_chats(state: tauri::State<'_, AppState>) -> Result<Vec<Chat>, String> {
    let live = live(&state).await?;
    let v = rpc(&live, "get_recommended_chats", json!({ "limit": 50 })).await?;
    Ok(v.pointer("/data/chats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| rpc_chat(c, &live.me))
        .filter(|c| !service::is_service_chat(c))
        .collect())
}

/// Joins a public group (`activate_chat`, as in the web client).
#[tauri::command]
pub async fn join_chat(state: tauri::State<'_, AppState>, chat_id: String) -> Result<Chat, String> {
    let live = live(&state).await?;
    let v = rpc(&live, "activate_chat", json!({ "chat_id": chat_id })).await?;
    let chat = v
        .pointer("/data/chat")
        .and_then(|c| rpc_chat(c, &live.me))
        .ok_or_else(|| "Telemost did not return the chat".to_owned())?;
    note_chat(&live, &chat);
    Ok(chat)
}
