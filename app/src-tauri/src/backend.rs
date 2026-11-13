//! Commands for the UI: connection, history, writes and live events.
//!
//! This file holds what the submodules share: the events for the UI, the state of the app and of
//! its connection, and the helpers for requests to Telemost. The commands are in the submodules,
//! one per subject.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use mostik_core::{Chat, ChatKind, Message, NotifyLevel, Snapshot, Timestamp, User, UserId};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::shape;
use mostik_yandex::web::{WebClient, WebError};
use mostik_yandex::wire::PushRequest;
use mostik_yandex::xiva_client::XivaClient;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use connection::Session;
use files::FileCache;

pub mod calls;
pub mod chats;
pub mod connection;
pub mod events;
pub mod files;
pub mod messages;
pub mod notify;
pub mod prefs;
pub mod presence;
pub mod service;
pub mod threads;

pub(crate) use calls::open_call;
pub(crate) use connection::{probing, write_probe};
pub(crate) use files::open_external;
pub use files::serve_file;
pub use presence::clear_meeting_status_later;

/// Live event for the UI.
pub const EVENT: &str = "mostik://event";
/// Open a message: the user clicked its notification.
pub const OPEN_EVENT: &str = "mostik://open";

/// The message a clicked notification was about.
#[derive(Clone, Serialize)]
pub struct OpenTarget {
    pub chat_id: String,
    pub ts: Timestamp,
    pub thread_parent: Option<Timestamp>,
}

impl OpenTarget {
    /// From the `extra` data [`notify`] gives a notification.
    pub fn from_extra(extra: &std::collections::HashMap<String, Value>) -> Option<Self> {
        Some(OpenTarget {
            chat_id: extra.get("chat")?.as_str()?.to_owned(),
            ts: extra.get("ts")?.as_u64()?,
            thread_parent: extra.get("thread").and_then(Value::as_u64),
        })
    }
}
/// Fresh snapshot after a reconnect.
pub const SNAPSHOT_EVENT: &str = "mostik://snapshot";
/// The session is no longer valid: the UI shows the login screen with this reason.
pub const SIGNED_OUT_EVENT: &str = "mostik://signed_out";

/// Result of `bootstrap` and `login`: either the login screen or a ready snapshot.
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Boot {
    NeedLogin { reason: Option<String> },
    Ready { snapshot: Box<Snapshot> },
}

/// State updated from live events.
#[derive(Default)]
struct Shared {
    me: Option<UserId>,
    /// Notifications paused on every device ("Do not disturb"): none are shown meanwhile.
    paused: Option<mostik_core::Pause>,
    /// The last `(Timestamp, SeqNo)` per chat, for read markers.
    last: BTreeMap<String, (Timestamp, u64)>,
    /// The read marker last sent per chat: the same one is not sent again (an own message is
    /// marked both by the send reply and by its echo).
    seen_sent: BTreeMap<String, (Timestamp, u64)>,
    /// What muted chats still notify about; chats not here notify about everything.
    muted: BTreeMap<String, NotifyLevel>,
    titles: BTreeMap<String, String>,
    names: BTreeMap<String, String>,
    /// Public group chats the user is in: `@<name>` mentions them too.
    channels: Vec<String>,
    /// Set once the snapshot is built: until then `muted`, `titles` and `names` are incomplete,
    /// so no notifications are shown.
    ready: bool,
    /// Events of chats that are not in the list yet, held until the chat itself is fetched, so
    /// the UI gets the chat first and then its messages in order.
    pending: BTreeMap<String, Vec<LiveEvent>>,
    /// Threads the user follows, as `(chat, parent)`: Telemost's list plus the ones the user
    /// has replied in since. Only replies in them notify, as in Telemost and Slack.
    followed: BTreeSet<(String, Timestamp)>,
    /// Threads the list did not have when it was last fetched: their replies need no new fetch.
    not_followed: BTreeSet<(String, Timestamp)>,
    /// Replies in threads of unknown standing that wait for the list to be fetched again, to
    /// notify about the ones the user turns out to follow.
    thread_wait: Vec<(Message, Option<User>)>,
    /// Mostik service chats (see [`service`]): their events never reach the UI.
    service: BTreeSet<String>,
    /// The service chats of the chat list, for the sync after the connection.
    service_chats: Vec<service::ServiceChat>,
    /// The user's organizations.
    orgs: Vec<u64>,
}

impl Shared {
    /// Records the names of `users`; a stand-in name is no name.
    fn learn_names<'a>(&mut self, users: impl IntoIterator<Item = &'a User>) {
        for u in users.into_iter().filter(|u| u.has_name()) {
            self.names.insert(u.id.0.clone(), u.display_name.clone());
        }
    }

    /// A chat the user is in from now on: `title` for its notifications, and its name for
    /// `@channel` mentions if it is public.
    fn note_chat(&mut self, chat: &Chat, title: String) {
        self.titles.insert(chat.id.0.clone(), title);
        let public = chat.kind == ChatKind::Group { public: true };
        if public && !chat.name.is_empty() && !self.channels.contains(&chat.name) {
            self.channels.push(chat.name.clone());
        }
    }

    /// Forgets the title of a chat the user has left, and its name among the `@channel` mentions.
    fn forget_title(&mut self, chat_id: &str) {
        if let Some(name) = self.titles.remove(chat_id) {
            // Titles keep a group's emoji as `⚠️ Test`, the channel list its raw name `⚠️|Test`.
            self.channels.retain(|c| mostik_core::group_label(c) != name);
        }
    }
}

/// Live connection: HTTP client, Xiva socket and shared state.
pub struct Live {
    files: StdMutex<FileCache>,
    /// Per-file locks of downloads in progress, so the same file is not fetched twice at once.
    downloads: StdMutex<std::collections::HashMap<String, Arc<Mutex<()>>>>,
    web: WebClient,
    xiva: XivaClient,
    me: UserId,
    user_agent: String,
    service_id: Option<String>,
    shared: Arc<StdMutex<Shared>>,
}

/// What notifies, from "Preferences": the UI sends `all`, `direct` or `nothing`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyMode {
    #[default]
    All,
    /// Direct chats and mentions.
    Direct,
    Nothing,
}

/// UI settings the backend needs (from "Preferences" in the UI).
#[derive(Clone)]
pub struct Prefs {
    pub notify: NotifyMode,
    pub preview: bool,
    pub english: bool,
    /// Notification sound, one of [`mostik_core::NOTIFICATION_SOUNDS`].
    pub sound: String,
}

impl Default for Prefs {
    fn default() -> Self {
        let english = std::env::var("LANG")
            .map(|l| !l.to_lowercase().starts_with("ru"))
            .unwrap_or(false);
        Self {
            notify: NotifyMode::All,
            preview: true,
            sound: "marimba".into(),
            english,
        }
    }
}

/// String in the UI language.
pub fn text(app: &AppHandle, ru: &'static str, en: &'static str) -> &'static str {
    let english = app
        .try_state::<AppState>()
        .and_then(|s| s.prefs.lock().ok().map(|p| p.english))
        .unwrap_or(false);
    if english { en } else { ru }
}

/// Tray menu items, kept to rename them when the language changes.
pub struct TrayItems {
    pub open: tauri::menu::MenuItem<tauri::Wry>,
    pub quit: tauri::menu::MenuItem<tauri::Wry>,
}

/// A file the user picked in the dialog or dropped onto the window: only such files may be sent.
#[derive(Clone, Serialize)]
pub struct PendingFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub image: bool,
}

fn pending_file(path: &std::path::Path) -> Option<PendingFile> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let image = mime_guess::from_path(path)
        .first()
        .is_some_and(|m| m.type_() == "image");
    Some(PendingFile {
        path: path.display().to_string(),
        name,
        size: meta.len(),
        image,
    })
}

/// Files dropped onto the window: the [`PendingFile`]s to attach.
pub const FILES_EVENT: &str = "mostik://files";
/// Files are dragged over the chat window: where, in physical pixels; `None` when the drag
/// leaves or ends.
pub const DRAG_EVENT: &str = "mostik://drag";

/// Files dropped onto the chat window and where, in physical pixels.
#[derive(Serialize)]
pub struct Dropped {
    pub files: Vec<PendingFile>,
    pub x: f64,
    pub y: f64,
}

/// Allows sending the files (dialog, drag and drop) and returns their description for the UI.
pub fn allow_files(app: &AppHandle, paths: &[std::path::PathBuf]) -> Vec<PendingFile> {
    let files: Vec<PendingFile> = paths.iter().filter_map(|p| pending_file(p)).collect();
    if let Some(state) = app.try_state::<AppState>()
        && let Ok(mut allowed) = state.allowed_files.lock()
    {
        allowed.extend(files.iter().map(|f| f.path.clone()));
    }
    files
}

#[derive(Default)]
pub struct AppState {
    pub prefs: StdMutex<Prefs>,
    allowed_files: StdMutex<BTreeSet<String>>,
    /// Contents of the files pasted or dropped into the message field, by their [`KEPT`](files::KEPT) path.
    kept_files: StdMutex<BTreeMap<String, bytes::Bytes>>,
    live: Mutex<Option<Arc<Live>>>,
    session: Session,
    reconnecting: AtomicBool,
    heartbeat_running: AtomicBool,
    diag_events: Arc<AtomicUsize>,
    pub service: service::State,
}

async fn live(state: &AppState) -> Result<Arc<Live>, String> {
    state
        .live
        .lock()
        .await
        .clone()
        .filter(|l| !l.xiva.is_closed())
        .ok_or_else(|| "No connection to Telemost, reconnecting…".to_owned())
}

/// Chat the request goes to: the chat itself or the thread chat of message `thread`.
fn target_chat(chat_id: &str, thread: Option<Timestamp>) -> Result<String, String> {
    match thread {
        None => Ok(chat_id.to_owned()),
        Some(ts) => {
            convert::thread_chat_id(chat_id, ts).ok_or_else(|| "Threads are not supported in this chat".to_owned())
        }
    }
}

async fn push(live: &Live, client_message: Value) -> Result<Value, String> {
    let sub = live
        .xiva
        .wait_subscription(Duration::from_secs(5))
        .await
        .ok_or("Telemost has not issued a subscription yet, try again in a couple of seconds")?;
    let req = PushRequest::new(&sub, &live.user_agent, client_message).with_origin(live.service_id.as_deref());
    let resp = live.xiva.call("push", &req, 20).await.map_err(|e| e.to_string())?;
    if resp.get("MessageInfo").is_none() && resp.get("Status").and_then(Value::as_u64).unwrap_or(0) != 0 {
        return Err(format!("Telemost rejected the request: {}", shape(&resp)));
    }
    Ok(resp)
}

/// Sends a client message over the current connection.
pub(crate) async fn push_now(app: &AppHandle, client_message: Value) -> Result<(), String> {
    let live = live(&app.state::<AppState>()).await?;
    push(&live, client_message).await.map(|_| ())
}

/// The person with their photo and name from the directory, when the connection is up.
pub(crate) async fn with_profile(app: &AppHandle, user: User) -> User {
    let mut users = [user];
    if let Ok(live) = live(&app.state::<AppState>()).await {
        fill_profiles(&live.web, &mut users).await;
    }
    let [user] = users;
    user
}

/// Photos, job titles and names from the directory: the history does not always carry
/// `AvatarId`, and `From` may hold only the GUID. Returns how many photos were filled in.
async fn fill_profiles(web: &WebClient, users: &mut [User]) -> usize {
    let guids: Vec<String> = users.iter().map(|u| u.id.0.clone()).collect();
    let mut filled = 0;
    let jobs = guids.chunks(convert::PROFILES_PER_REQUEST).map(|chunk| {
        let mut web = web.clone_light();
        async move { web.rpc("get_users_data", json!({ "guids": chunk })).await }
    });
    for v in futures_util::future::join_all(jobs).await.into_iter().flatten() {
        for p in convert::profiles(&v) {
            if let Some(u) = users.iter_mut().find(|u| u.id == p.id) {
                if u.avatar.is_none() && p.avatar.is_some() {
                    u.avatar = p.avatar.clone();
                    filled += 1;
                }
                if u.title.is_none() {
                    u.title = p.position.clone();
                }
                if !p.display_name.is_empty() && !u.has_name() {
                    u.display_name = p.display_name.clone();
                }
            }
        }
    }
    filled
}

/// An API error for the UI: a busy or unreachable Telemost gets a plain explanation.
fn web_error(app: &AppHandle, e: WebError) -> String {
    if e.is_transient() {
        text(
            app,
            "Телемост не отвечает, попробуйте ещё раз чуть позже",
            "Telemost isn't responding, try again in a moment",
        )
        .to_owned()
    } else {
        e.to_string()
    }
}

/// HTTP RPC call `method` that must succeed: a network error and a refusal are both errors.
async fn rpc(live: &Live, method: &str, params: Value) -> Result<Value, String> {
    let v = live
        .web
        .clone_light()
        .rpc(method, params)
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
    Ok(v)
}

fn rpc_ok(v: &Value) -> Result<(), String> {
    if v.get("status").and_then(Value::as_str) == Some("ok") {
        Ok(())
    } else {
        let code = v.pointer("/data/code").and_then(Value::as_str).unwrap_or("?");
        let text = v.pointer("/data/text").and_then(Value::as_str).unwrap_or("");
        Err(format!("Telemost: {code} {text}").trim().to_owned())
    }
}
