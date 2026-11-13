//! Tauri command calls and event subscriptions. Without Tauri (in a browser) the UI runs on demo data.

use mostik_core::{Chat, ChatId, Message, Reaction, Snapshot, Timestamp, UnreadBadge, User, UserId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    /// `invoke` with options: raw bytes as the arguments and request headers.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke_with(cmd: &str, args: JsValue, options: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn tauri_listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue>;
}

/// Whether the UI runs inside Tauri (otherwise it is the browser demo).
pub fn available() -> bool {
    js_sys::Reflect::has(&js_sys::global(), &JsValue::from_str("__TAURI__")).unwrap_or(false)
}

async fn call<T: DeserializeOwned>(cmd: &str, args: impl Serialize) -> Result<T, String> {
    // In the browser demo there is no backend: an error instead of a JS exception that kills the UI.
    if !available() {
        return Err(format!("{cmd}: no Tauri backend"));
    }
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| e.to_string())?;
    let value = tauri_invoke(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| format!("{cmd}: {e}"))
}

/// Subscribes to a backend event for the whole lifetime of the window.
pub async fn listen<T: DeserializeOwned + 'static>(event: &str, mut handler: impl FnMut(T) + 'static) {
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        let payload = js_sys::Reflect::get(&e, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
        match serde_wasm_bindgen::from_value::<T>(payload) {
            Ok(v) => handler(v),
            Err(err) => leptos::logging::warn!("event not parsed: {err}"),
        }
    });
    if let Err(e) = tauri_listen(event, &closure).await {
        leptos::logging::warn!("listen {event}: {e:?}");
    }
    closure.forget();
}

/// Mirror of the backend `Boot`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Boot {
    NeedLogin { reason: Option<String> },
    Ready { snapshot: Box<Snapshot> },
}

/// Mirror of `mostik_yandex::convert::LiveEvent`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    Upsert {
        message: Box<Message>,
        author: Option<User>,
        #[serde(default)]
        mentioned: Vec<User>,
    },
    Deleted {
        chat_id: ChatId,
        ts: Timestamp,
        #[serde(default)]
        thread_parent: Option<Timestamp>,
    },
    Typing {
        chat_id: ChatId,
        user: UserId,
    },
    Reactions {
        chat_id: ChatId,
        ts: Timestamp,
        reactions: Vec<Reaction>,
        #[serde(default)]
        users: Vec<User>,
    },
    Seen {
        chat_id: ChatId,
    },
    PeerSeen {
        chat_id: ChatId,
        user: UserId,
        ts: Timestamp,
        reads: Option<mostik_core::Reads>,
    },
    ThreadSeen {
        chat_id: ChatId,
        parent: Timestamp,
    },
    Online {
        user: UserId,
    },
    Status(mostik_core::PresenceInfo),
    Pinned {
        chat_id: ChatId,
        ts: Option<Timestamp>,
    },
    ChatAdded {
        chat: Chat,
        section: String,
        users: Vec<User>,
    },
    Connection {
        online: bool,
    },
    Users {
        users: Vec<User>,
    },
    Threads {
        threads: Vec<mostik_core::FollowedThread>,
        users: Vec<User>,
    },
    ChatRemoved {
        chat_id: ChatId,
    },
    ChatChanged {
        chat_id: ChatId,
        name: Option<String>,
        description: Option<String>,
        member_count: Option<u32>,
    },
    Folders {
        folders: Vec<mostik_core::ChatFolder>,
    },
    Starred {
        chats: Vec<ChatId>,
    },
    Mutings {
        chats: Vec<(ChatId, mostik_core::NotifyLevel)>,
    },
    Meeting(mostik_core::Meeting),
    Paused {
        pause: Option<mostik_core::Pause>,
    },
}

pub const EVENT: &str = "mostik://event";
pub const SNAPSHOT_EVENT: &str = "mostik://snapshot";
/// The session expired while the app was running: show the login screen with this reason.
pub const SIGNED_OUT_EVENT: &str = "mostik://signed_out";
/// The current call changed: [`CallInfo`] or `null`.
pub const CALL_EVENT: &str = "mostik://call";
/// The call ringing for the user changed: [`Ring`] or `null`.
pub const RING_EVENT: &str = "mostik://ring";

/// Mirror of the backend `CallInfo`: the call window that is open now.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallInfo {
    pub chat_id: Option<String>,
    pub label: String,
}

/// A new [`UpdateStatus`]: an update was found or is being installed.
pub const UPDATE_EVENT: &str = "mostik://update";

/// Mirror of the backend `update::UpdateStatus`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    #[default]
    None,
    Unsupported,
    Available {
        version: String,
    },
    Installing {
        version: String,
        percent: Option<u8>,
    },
}

#[derive(Serialize)]
struct NoArgs {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatArgs<'a> {
    chat_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryArgs<'a> {
    chat_id: &'a str,
    before: Option<Timestamp>,
    thread: Option<Timestamp>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendArgs<'a> {
    chat_id: &'a str,
    text: &'a str,
    thread: Option<Timestamp>,
    reply: Option<ReplyRef<'a>>,
}

/// The message a sent one replies to, in the same chat or thread, and the quoted fragment.
#[derive(Serialize)]
pub struct ReplyRef<'a> {
    pub ts: Timestamp,
    pub quote: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EditArgs<'a> {
    chat_id: &'a str,
    ts: Timestamp,
    text: &'a str,
    thread: Option<Timestamp>,
    /// The message has more than text: the backend resends all of it.
    whole: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MsgArgs<'a> {
    chat_id: &'a str,
    ts: Timestamp,
    thread: Option<Timestamp>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReactionArgs<'a> {
    chat_id: &'a str,
    ts: Timestamp,
    kind: u64,
    add: bool,
    thread: Option<Timestamp>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportantArgs<'a> {
    chat_id: &'a str,
    ts: Timestamp,
    important: bool,
    thread: Option<Timestamp>,
}

pub async fn bootstrap() -> Result<Boot, String> {
    call("bootstrap", NoArgs {}).await
}

pub async fn login() -> Result<Boot, String> {
    call("login", NoArgs {}).await
}

pub async fn logout() -> Result<(), String> {
    call("logout", NoArgs {}).await
}

pub async fn update_status() -> Result<UpdateStatus, String> {
    call("update_status", NoArgs {}).await
}

#[derive(Serialize)]
struct AutoUpdateArgs {
    enabled: bool,
}

pub async fn set_auto_update(enabled: bool) -> Result<(), String> {
    call("set_auto_update", AutoUpdateArgs { enabled }).await
}

/// Checks for a new version now.
pub async fn check_update() -> Result<UpdateStatus, String> {
    call("check_update", NoArgs {}).await
}

/// Installs the found update and restarts the app; returns only on failure.
pub async fn install_update() -> Result<(), String> {
    call("install_update", NoArgs {}).await
}

/// A history page and the authors of its messages the app has not seen yet.
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    pub users: Vec<User>,
    /// The server may have older messages.
    pub more: bool,
    /// The time of the oldest record of the page, deleted messages and service records included:
    /// the next older page starts before it.
    #[serde(default)]
    pub oldest: Option<Timestamp>,
    /// How far the others have read the chat (`LastSeenTsMcs`); 0 for a thread.
    #[serde(default)]
    pub peer_seen: Timestamp,
}

pub async fn history(
    chat_id: &str,
    before: Option<Timestamp>,
    thread: Option<Timestamp>,
) -> Result<HistoryPage, String> {
    call(
        "history",
        HistoryArgs {
            chat_id,
            before,
            thread,
        },
    )
    .await
}

pub async fn mark_read(chat_id: &str) -> Result<(), String> {
    call("mark_read", ChatArgs { chat_id }).await
}

/// Followed threads and the people they mention.
#[derive(Debug, Clone, Deserialize)]
pub struct Threads {
    pub threads: Vec<mostik_core::FollowedThread>,
    pub users: Vec<User>,
}

pub async fn followed_threads() -> Result<Threads, String> {
    call("followed_threads", NoArgs {}).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadReadArgs<'a> {
    chat_id: &'a str,
    parent: Timestamp,
    ts: Timestamp,
    seq_no: u64,
}

pub async fn mark_thread_read(chat_id: &str, parent: Timestamp, ts: Timestamp, seq_no: u64) -> Result<(), String> {
    call(
        "mark_thread_read",
        ThreadReadArgs {
            chat_id,
            parent,
            ts,
            seq_no,
        },
    )
    .await
}

pub async fn send_message(
    chat_id: &str,
    text: &str,
    thread: Option<Timestamp>,
    reply: Option<ReplyRef<'_>>,
) -> Result<Message, String> {
    call(
        "send_message",
        SendArgs {
            chat_id,
            text,
            thread,
            reply,
        },
    )
    .await
}

pub async fn edit_message(m: &Message, text: &str) -> Result<(), String> {
    call(
        "edit_message",
        EditArgs {
            chat_id: &m.chat_id.0,
            ts: m.ts,
            text,
            thread: m.thread_parent,
            whole: !m.attachments.is_empty() || !m.forwarded.is_empty(),
        },
    )
    .await
}

pub async fn delete_message(m: &Message) -> Result<(), String> {
    call(
        "delete_message",
        MsgArgs {
            chat_id: &m.chat_id.0,
            ts: m.ts,
            thread: m.thread_parent,
        },
    )
    .await
}

pub async fn set_reaction(m: &Message, kind: u64, add: bool) -> Result<(), String> {
    call(
        "set_reaction",
        ReactionArgs {
            chat_id: &m.chat_id.0,
            ts: m.ts,
            kind,
            add,
            thread: m.thread_parent,
        },
    )
    .await
}

pub async fn set_important(m: &Message, important: bool) -> Result<(), String> {
    call(
        "set_important",
        ImportantArgs {
            chat_id: &m.chat_id.0,
            ts: m.ts,
            important,
            thread: m.thread_parent,
        },
    )
    .await
}

pub async fn typing(chat_id: &str) -> Result<(), String> {
    call("typing", ChatArgs { chat_id }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileArgs<'a> {
    file_id: &'a str,
    name: &'a str,
}

/// Downloads an attachment to "Downloads" and opens it; returns the path.
pub async fn save_file(file_id: &str, name: &str) -> Result<String, String> {
    call("save_file", FileArgs { file_id, name }).await
}

/// Attachment URL for `<img>`: the backend serves it over the `mostikfile` scheme.
pub fn file_url(file_id: &str, size: Option<&str>) -> String {
    let id = String::from(js_sys::encode_uri_component(file_id));
    let windows = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains("Windows"));
    let base = if windows {
        "http://mostikfile.localhost"
    } else {
        "mostikfile://localhost"
    };
    match size {
        Some(s) => format!("{base}/{id}?size={s}"),
        None => format!("{base}/{id}"),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrefsArgs<'a> {
    notify: &'static str,
    preview: bool,
    lang: &'static str,
    sound: &'a str,
}

/// Passes to the backend what affects notifications and the tray.
pub async fn set_preferences(s: &crate::settings::Settings) -> Result<(), String> {
    use crate::settings::NotifyMode;
    let notify = match s.notify {
        NotifyMode::All => "all",
        NotifyMode::DirectAndMentions => "direct",
        NotifyMode::Nothing => "nothing",
    };
    call(
        "set_preferences",
        PrefsArgs {
            notify,
            preview: s.notify_preview,
            lang: s.lang.code(),
            sound: &s.sound,
        },
    )
    .await
}

#[derive(Serialize)]
struct SoundArgs<'a> {
    id: &'a str,
}

/// Plays a notification sound (the backend plays them).
pub async fn play_sound(id: &str) -> Result<(), String> {
    call("play_sound", SoundArgs { id }).await
}

#[derive(Serialize)]
struct TitleArgs<'a> {
    title: &'a str,
}

/// Application window title (document.title does not affect the window in WebKitGTK).
pub async fn set_window_title(title: &str) -> Result<(), String> {
    call("set_window_title", TitleArgs { title }).await
}

#[derive(Serialize)]
struct BadgeArgs {
    badge: UnreadBadge,
    count: u32,
}

/// The unread mark on the tray icon, the taskbar button or the Dock icon.
pub async fn set_unread_badge(badge: UnreadBadge, count: u32) -> Result<(), String> {
    call("set_unread_badge", BadgeArgs { badge, count }).await
}

#[derive(Serialize)]
struct ZoomArgs {
    scale: f64,
}

/// Window zoom: in the app it is the webview zoom, like Ctrl+= in Slack.
pub async fn set_zoom(percent: u32) -> Result<(), String> {
    call(
        "set_zoom",
        ZoomArgs {
            scale: f64::from(percent) / 100.0,
        },
    )
    .await
}

#[derive(Serialize)]
struct GuidArgs<'a> {
    guid: &'a str,
}

pub async fn user_profile(guid: &str) -> Result<mostik_core::UserProfile, String> {
    call("user_profile", GuidArgs { guid }).await
}

pub async fn open_dm(guid: &str) -> Result<mostik_core::Chat, String> {
    call("open_dm", GuidArgs { guid }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchArgs<'a> {
    query: &'a str,
    chat_id: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateArgs<'a> {
    name: &'a str,
    description: &'a str,
    members: Vec<String>,
    public: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MembersArgs<'a> {
    chat_id: &'a str,
    guids: Vec<String>,
    add: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateArgs<'a> {
    chat_id: &'a str,
    name: &'a str,
    description: &'a str,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChatDetails {
    pub name: String,
    pub description: String,
    pub role: String,
    pub member_count: u64,
}

pub async fn search_people(query: &str, chat_id: Option<&str>) -> Result<Vec<mostik_core::UserProfile>, String> {
    call("search_people", SearchArgs { query, chat_id }).await
}

pub async fn create_channel(
    name: &str,
    description: &str,
    members: Vec<String>,
    public: bool,
) -> Result<mostik_core::Chat, String> {
    call(
        "create_channel",
        CreateArgs {
            name,
            description,
            members,
            public,
        },
    )
    .await
}

pub async fn chat_members(chat_id: &str) -> Result<Vec<mostik_core::ChatMember>, String> {
    call("chat_members", ChatArgs { chat_id }).await
}

pub async fn change_members(chat_id: &str, guids: Vec<String>, add: bool) -> Result<(), String> {
    call("change_members", MembersArgs { chat_id, guids, add }).await
}

pub async fn update_chat(chat_id: &str, name: &str, description: &str) -> Result<ChatDetails, String> {
    call(
        "update_chat",
        UpdateArgs {
            chat_id,
            name,
            description,
        },
    )
    .await
}

pub async fn leave_chat(chat_id: &str) -> Result<(), String> {
    call("leave_chat", ChatArgs { chat_id }).await
}

pub async fn chat_details(chat_id: &str) -> Result<ChatDetails, String> {
    call("chat_details", ChatArgs { chat_id }).await
}

/// A file picked in the dialog, dropped onto the window or pasted.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PendingFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub image: bool,
}

pub const FILES_EVENT: &str = "mostik://files";
/// Files are dragged over the window: where, in physical pixels; `None` when the drag ends.
pub const DRAG_EVENT: &str = "mostik://drag";

/// Files dropped onto the window and where, in physical pixels.
#[derive(Debug, Clone, Deserialize)]
pub struct Dropped {
    pub files: Vec<PendingFile>,
    pub x: f64,
    pub y: f64,
}
/// The custom emoji sets changed.
pub const EMOJI_EVENT: &str = "mostik://emoji";
/// The backend showed a notification: play the sound.
/// The user clicked a message notification: open that message.
pub const OPEN_EVENT: &str = "mostik://open";

#[derive(Debug, Clone, Deserialize)]
pub struct OpenTarget {
    pub chat_id: ChatId,
    pub ts: Timestamp,
    pub thread_parent: Option<Timestamp>,
}

/// A file taken off the message: the app no longer keeps a pasted one.
pub async fn forget_file(path: &str) -> Result<(), String> {
    #[derive(Serialize)]
    struct Args<'a> {
        path: &'a str,
    }
    call("forget_file", Args { path }).await
}

pub async fn pick_files() -> Result<Vec<PendingFile>, String> {
    call("pick_files", NoArgs {}).await
}

/// Hands the contents of a pasted or dropped file to the app, which keeps it to be sent like a
/// picked one. The bytes go as they are, not as JSON.
pub async fn keep_file(name: &str, data: &js_sys::ArrayBuffer) -> Result<PendingFile, String> {
    if !available() {
        return Err("keep_file: no Tauri backend".into());
    }
    let headers = js_sys::Object::new();
    let name = js_sys::encode_uri_component(name);
    js_sys::Reflect::set(&headers, &"x-file-name".into(), &name).map_err(|e| format!("{e:?}"))?;
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"headers".into(), &headers).map_err(|e| format!("{e:?}"))?;
    let body = js_sys::Uint8Array::new(data);
    let value = tauri_invoke_with("keep_file", body.into(), options.into())
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| format!("keep_file: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendFilesArgs<'a> {
    chat_id: &'a str,
    thread: Option<Timestamp>,
    paths: Vec<String>,
    text: &'a str,
}

pub async fn send_files(
    chat_id: &str,
    thread: Option<Timestamp>,
    paths: Vec<String>,
    text: &str,
) -> Result<Vec<Message>, String> {
    call(
        "send_files",
        SendFilesArgs {
            chat_id,
            thread,
            paths,
            text,
        },
    )
    .await
}

#[derive(Serialize)]
struct GuidsArgs {
    guids: Vec<String>,
}

pub async fn presence(guids: Vec<String>) -> Result<Vec<mostik_core::PresenceInfo>, String> {
    call("presence", GuidsArgs { guids }).await
}

#[derive(Serialize)]
struct StatusArgs<'a> {
    availability: u64,
    emoji: &'a str,
    text: &'a str,
    duration: Option<u64>,
}

pub async fn set_status(availability: u64, emoji: &str, text: &str, duration: Option<u64>) -> Result<(), String> {
    call(
        "set_status",
        StatusArgs {
            availability,
            emoji,
            text,
            duration,
        },
    )
    .await
}

pub async fn test_notification() -> Result<(), String> {
    call("test_notification", NoArgs {}).await
}

pub async fn open_yandex_id() -> Result<(), String> {
    call("open_yandex_id", NoArgs {}).await
}

#[derive(Serialize)]
struct UrlArgs<'a> {
    url: &'a str,
}

pub async fn open_url(url: &str) -> Result<(), String> {
    call("open_url", UrlArgs { url }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryArgs<'a> {
    query: &'a str,
    chat_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FoundMessages {
    pub messages: Vec<Message>,
    pub users: Vec<User>,
}

/// Server-side message search in the listed chats.
pub async fn search_messages(query: &str, chat_ids: Vec<String>) -> Result<FoundMessages, String> {
    call("search_messages", QueryArgs { query, chat_ids }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NotifyLevelArgs<'a> {
    chat_id: &'a str,
    level: mostik_core::NotifyLevel,
}

pub async fn set_notify_level(chat_id: &str, level: mostik_core::NotifyLevel) -> Result<(), String> {
    call("set_notify_level", NotifyLevelArgs { chat_id, level }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StarArgs<'a> {
    chat_id: &'a str,
    starred: bool,
}

#[derive(Serialize)]
struct FoldersArgs<'a> {
    org: &'a str,
    folders: &'a [mostik_core::ChatFolder],
}

/// Writes the organization's chat folders; returns those of every organization.
pub async fn save_chat_folders(
    org: &str,
    folders: &[mostik_core::ChatFolder],
) -> Result<Vec<mostik_core::ChatFolder>, String> {
    call("save_chat_folders", FoldersArgs { org, folders }).await
}

/// Returns the new list of starred chats.
pub async fn set_starred(chat_id: &str, starred: bool) -> Result<Vec<String>, String> {
    call("set_starred", StarArgs { chat_id, starred }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnreadArgs<'a> {
    chat_id: &'a str,
    ts: Timestamp,
    seq_no: u64,
}

pub async fn mark_unread(chat_id: &str, ts: Timestamp, seq_no: u64) -> Result<(), String> {
    call("mark_unread", UnreadArgs { chat_id, ts, seq_no }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PinArgs<'a> {
    chat_id: &'a str,
    ts: Option<Timestamp>,
}

pub async fn pin_message(chat_id: &str, ts: Option<Timestamp>) -> Result<(), String> {
    call("pin_message", PinArgs { chat_id, ts }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardArgs<'a> {
    from_chat: &'a str,
    ts: Timestamp,
    thread: Option<Timestamp>,
    to_chat: &'a str,
    text: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Sent {
    pub ts: Timestamp,
    pub seq_no: u64,
}

pub async fn forward_message(m: &Message, to_chat: &str, text: &str) -> Result<Sent, String> {
    call(
        "forward_message",
        ForwardArgs {
            from_chat: &m.chat_id.0,
            ts: m.ts,
            thread: m.thread_parent,
            to_chat,
            text,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CallArgs<'a> {
    chat_id: &'a str,
    label: &'a str,
    url: Option<&'a str>,
}

/// `url` joins a call already going on in the chat instead of starting one.
pub async fn start_call(chat_id: &str, label: &str, url: Option<&str>) -> Result<(), String> {
    call("start_call", CallArgs { chat_id, label, url }).await
}

pub async fn focus_call() -> Result<(), String> {
    call("focus_call", NoArgs {}).await
}

pub async fn leave_call() -> Result<(), String> {
    call("leave_call", NoArgs {}).await
}

/// A call ringing for the user, for the incoming call dialog (mirror of the backend `ring::Ring`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ring {
    pub caller: User,
}

pub async fn ring_info() -> Result<Option<Ring>, String> {
    call("ring_info", NoArgs {}).await
}

#[derive(Serialize)]
struct AnswerArgs {
    accept: bool,
}

/// "Accept" or "Decline" in the incoming call dialog.
pub async fn answer_ring(accept: bool) -> Result<(), String> {
    call("answer_ring", AnswerArgs { accept }).await
}

pub async fn current_call() -> Result<Option<CallInfo>, String> {
    call("current_call", NoArgs {}).await
}

pub async fn recommended_chats() -> Result<Vec<mostik_core::Chat>, String> {
    call("recommended_chats", NoArgs {}).await
}

pub async fn join_chat(chat_id: &str) -> Result<mostik_core::Chat, String> {
    call("join_chat", ChatArgs { chat_id }).await
}

/// Custom emoji sets of the user's organizations (from the disk cache until the chats are read).
pub async fn custom_emoji() -> Result<Vec<mostik_core::custom_emoji::EmojiSet>, String> {
    call("custom_emoji", ()).await
}

/// Picks an image for a new custom emoji.
pub async fn pick_emoji_image() -> Result<Option<PendingFile>, String> {
    call("pick_emoji_image", ()).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewEmojiArgs<'a> {
    emoji: NewEmoji<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewEmoji<'a> {
    path: &'a str,
    name: &'a str,
    org: Option<u64>,
}

/// Adds a custom emoji to the service chat of organization `org`; returns the new sets.
pub async fn add_custom_emoji(
    path: &str,
    name: &str,
    org: Option<u64>,
) -> Result<Vec<mostik_core::custom_emoji::EmojiSet>, String> {
    call(
        "add_custom_emoji",
        NewEmojiArgs {
            emoji: NewEmoji { path, name, org },
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EmojiArgs<'a> {
    chat_id: &'a str,
    ts: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// Renames an own custom emoji; returns the new sets.
pub async fn rename_custom_emoji(
    chat_id: &str,
    ts: Timestamp,
    name: &str,
) -> Result<Vec<mostik_core::custom_emoji::EmojiSet>, String> {
    call(
        "rename_custom_emoji",
        EmojiArgs {
            chat_id,
            ts,
            name: Some(name),
        },
    )
    .await
}

/// Deletes an own custom emoji; returns the new sets.
pub async fn delete_custom_emoji(
    chat_id: &str,
    ts: Timestamp,
) -> Result<Vec<mostik_core::custom_emoji::EmojiSet>, String> {
    call(
        "delete_custom_emoji",
        EmojiArgs {
            chat_id,
            ts,
            name: None,
        },
    )
    .await
}

/// A file the user has just picked, for a preview: the backend serves only picked files.
pub fn local_file_url(path: &str) -> String {
    file_url(&format!("local/{path}"), None)
}

/// Image of a custom emoji: the backend serves it from its disk cache.
pub fn emoji_url(file_id: &str) -> String {
    file_url(&format!("emoji/{file_id}"), None)
}
