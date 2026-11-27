//! Commands for the UI: connection, history, writes and live events.
//!
//! This file holds what the submodules share: the events for the UI, the state of the app and of
//! its connection, and the helpers for requests to Telemost. The commands are in the submodules,
//! one per subject.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use mostik_core::{Chat, ChatId, ChatKind, Message, NotifyLevel, Timestamp, User, UserId};
pub use mostik_ipc::{Boot, Dropped, OpenTarget, PendingFile};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::shape;
use mostik_yandex::web::{WebClient, WebError};
use mostik_yandex::wire::PushRequest;
use mostik_yandex::xiva_client::XivaClient;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use connection::Session;
use files::{FileCache, note_files};

pub mod calls;
pub mod chats;
pub mod connection;
pub mod events;
pub mod files;
pub mod messages;
pub mod notify;
pub mod prefs;
pub mod presence;
pub mod report;
pub mod service;
pub mod threads;
pub mod voice;

pub(crate) use calls::open_call;
pub(crate) use connection::{probing, write_probe};
pub(crate) use files::open_external;
pub use files::{serve_file, served_type};
pub use presence::clear_meeting_status_later;

/// Live event for the UI.
pub const EVENT: &str = "mostik://event";
/// Open a message: the user clicked its notification.
pub const OPEN_EVENT: &str = "mostik://open";

/// The message a clicked notification was about, from the `extra` data [`notify`] gives the
/// notification.
pub fn open_target(extra: &std::collections::HashMap<String, Value>) -> Option<OpenTarget> {
    Some(OpenTarget {
        chat_id: ChatId(extra.get("chat")?.as_str()?.to_owned()),
        ts: extra.get("ts")?.as_u64()?,
        thread_parent: extra.get("thread").and_then(Value::as_u64),
    })
}
/// Fresh snapshot after a reconnect.
pub const SNAPSHOT_EVENT: &str = "mostik://snapshot";
/// The session is no longer valid: the UI shows the login screen with this reason.
pub const SIGNED_OUT_EVENT: &str = "mostik://signed_out";

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
    /// Threads Telemost says the user follows now (`you_added_to_thread`, for a mention, say)
    /// that the fetched list did not have yet: their replies notify as followed ones. A list that
    /// has one takes it over into `followed`.
    joined: BTreeSet<(String, Timestamp)>,
    /// Threads Telemost said the user stopped following (`you_removed_from_thread`) on this
    /// connection. No fetched list brings them back, since it may have gone out before; only
    /// Telemost's word does: the user's own reply or a mention there, or `you_added_to_thread`.
    left: BTreeSet<(String, Timestamp)>,
    /// Replies in threads of unknown standing that wait for the list to be fetched again, to
    /// notify about the ones the user turns out to follow.
    thread_wait: Vec<(Message, Option<User>, mostik_core::event::Alert)>,
    /// Mostik service chats (see [`service`]): their events never reach the UI.
    service: BTreeSet<String>,
    /// The service chats of the chat list, for the sync after the connection.
    service_chats: Vec<service::ServiceChat>,
    /// The user's organizations.
    orgs: Vec<u64>,
    /// Messages that came while the connection before this one was down (see
    /// [`notify::missed`]): they notify once this connection is in place.
    missed: Vec<(Message, mostik_core::event::Alert)>,
    /// The latest `ReactionsVersion` per message, `(chat, ts)`: a push with the reactions of an
    /// older one, come late, does not undo the newer ones (see `events::sync_shared`).
    reactions_seen: BTreeMap<(String, Timestamp), u64>,
}

impl Shared {
    /// Records the names of `users`; a stand-in name is no name.
    fn learn_names<'a>(&mut self, users: impl IntoIterator<Item = &'a User>) {
        for u in users.into_iter().filter(|u| u.has_name()) {
            self.names.insert(u.id.0.clone(), u.display_name.clone());
        }
    }

    /// A chat the user is in from now on: `title` for its notifications, and its name for
    /// `@channel` mentions if it is public. A stand-in title (none, or an unknown person's) does
    /// not replace a real one: two fetches of a new chat may end in any order.
    fn note_chat(&mut self, chat: &Chat, title: String) {
        let stand_in = title.is_empty() || title == mostik_core::UNKNOWN_USER;
        let known = self
            .titles
            .get(&chat.id.0)
            .is_some_and(|t| !t.is_empty() && t != mostik_core::UNKNOWN_USER);
        if !(stand_in && known) {
            self.titles.insert(chat.id.0.clone(), title);
        }
        let public = chat.kind == ChatKind::Group { public: true };
        if public && !chat.name.is_empty() && !self.channels.contains(&chat.name) {
            self.channels.push(chat.name.clone());
        }
    }

    /// Whether the user follows thread `key`: by the list, or by Telemost's word since it was fetched.
    fn follows(&self, key: &(String, Timestamp)) -> bool {
        self.followed.contains(key) || self.joined.contains(key)
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

pub use mostik_ipc::cmd::{NotifyMode, TrayIcon};

/// UI settings the backend needs (from "Preferences" in the UI).
#[derive(Clone)]
pub struct Prefs {
    pub notify: NotifyMode,
    pub preview: bool,
    pub english: bool,
    /// Notification sound, one of [`mostik_core::NOTIFICATION_SOUNDS`].
    pub sound: String,
    /// The tray icon on Linux and Windows (see [`crate::badge`]).
    pub tray_icon: TrayIcon,
    /// Closing the main window hides it to the tray (Linux and Windows).
    pub close_to_tray: bool,
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
            tray_icon: if cfg!(target_os = "linux") {
                TrayIcon::White
            } else {
                TrayIcon::App
            },
            close_to_tray: false,
        }
    }
}

/// String in the UI language.
pub fn text(app: &AppHandle, ru: &'static str, en: &'static str) -> &'static str {
    if english(app) { en } else { ru }
}

/// Whether the UI is in English rather than Russian.
pub fn english(app: &AppHandle) -> bool {
    app.try_state::<AppState>()
        .and_then(|s| s.prefs.lock().ok().map(|p| p.english))
        .unwrap_or(false)
}

/// Tray menu items, kept to rename them when the language changes.
pub struct TrayItems {
    pub open: tauri::menu::MenuItem<tauri::Wry>,
    pub quit: tauri::menu::MenuItem<tauri::Wry>,
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
/// Files are dragged over the chat window: where, in logical pixels of the window; `None` when
/// the drag leaves or ends.
pub const DRAG_EVENT: &str = "mostik://drag";

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
    /// The unread mark on the tray icon, kept to draw it again in another colour.
    pub tray_badge: StdMutex<mostik_core::UnreadBadge>,
    allowed_files: StdMutex<BTreeSet<String>>,
    /// Contents of the files pasted or dropped into the message field, by their [`KEPT`](files::KEPT) path.
    kept_files: StdMutex<BTreeMap<String, bytes::Bytes>>,
    /// The attachments the UI may save (see [`files::KnownFiles`]): kept across reconnections, as
    /// the UI keeps the messages it has loaded.
    known_files: StdMutex<files::KnownFiles>,
    /// How many attachments are being saved now.
    saving: AtomicUsize,
    /// The uploads of the sendings going on.
    uploads: StdMutex<files::Uploads>,
    live: Mutex<Option<Arc<Live>>>,
    session: Session,
    reconnecting: AtomicBool,
    heartbeat_running: AtomicBool,
    diag_events: Arc<AtomicUsize>,
    pub service: service::State,
    /// What the connection that went down had seen, until a new one is up (see [`notify::Gap`]).
    gap: StdMutex<Option<notify::Gap>>,
    /// The people the snapshots fetched lately (see [`fill_profiles_cached`]).
    profiles: StdMutex<ProfileCache>,
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

/// How a message of a sending goes, as the web client sends them: the first as the user chose
/// (without a sound, important), the later ones (the rest of a long text, more files) with no
/// notification and not marked, so one sending does not ring many times.
fn sending_options(options: mostik_ipc::cmd::SendOptions, first: bool) -> (mostik_yandex::wire::Notify, bool) {
    use mostik_yandex::wire;
    match (first, options.silent) {
        (false, _) => (wire::Notify::Silent, false),
        (true, true) => (wire::Notify::Muted, options.important),
        (true, false) => (wire::Notify::Default, options.important),
    }
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
    rejection(&resp).map_or(Ok(resp), Err)
}

/// Why Telemost refused a push, from its answer: an error status with no message info, or with an
/// empty one, as for a text too big (`{"Status":13,"Details":"message too big: …","MessageInfo":{}}`).
fn rejection(resp: &Value) -> Option<String> {
    let status = resp.get("Status").and_then(Value::as_u64).unwrap_or(0);
    let info = resp.get("MessageInfo");
    let empty = info.and_then(Value::as_object).is_some_and(serde_json::Map::is_empty);
    let refused = (info.is_none() && status != 0) || (empty && status > 1);
    if !refused {
        return None;
    }
    let details = resp.get("Details").and_then(Value::as_str).unwrap_or_default();
    if details.starts_with("message too big") {
        return Some(mostik_ipc::cmd::MESSAGE_TOO_LONG.to_owned());
    }
    let why = if details.is_empty() {
        shape(resp).to_string()
    } else {
        details.to_owned()
    };
    Some(format!("Telemost rejected the request: {why}"))
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
    apply_profiles(users, &fetch_profiles(web, &guids).await.0)
}

/// [`fill_profiles`] for the snapshot of a connection of session `generation`: the people the
/// session's connections fetched in the last [`PROFILE_TTL`] are not fetched again. Telemost
/// drops the connection a few times an hour, and each new one would fetch the same people.
async fn fill_profiles_cached(state: &AppState, generation: u64, web: &WebClient, users: &mut [User]) -> usize {
    let now = std::time::Instant::now();
    let mut known = Vec::new();
    let mut missing = Vec::new();
    if let Ok(mut cache) = state.profiles.lock() {
        if cache.generation != generation {
            *cache = ProfileCache {
                generation,
                ..ProfileCache::default()
            };
        }
        cache.fetched.retain(|_, (at, _)| now.duration_since(*at) < PROFILE_TTL);
        cache.absent.retain(|_, at| now.duration_since(*at) < PROFILE_TTL);
        for u in users.iter() {
            match cache.fetched.get(&u.id.0) {
                Some((_, p)) => known.push(p.clone()),
                None if cache.absent.contains_key(&u.id.0) => {}
                None => missing.push(u.id.0.clone()),
            }
        }
    }
    let (fetched, answered) = fetch_profiles(web, &missing).await;
    if let Ok(mut cache) = state.profiles.lock()
        && cache.generation == generation
    {
        for p in &fetched {
            cache.fetched.insert(p.id.0.clone(), (now, p.clone()));
        }
        for guid in answered.into_iter().filter(|g| !fetched.iter().any(|p| &p.id.0 == g)) {
            cache.absent.insert(guid, now);
        }
    }
    known.extend(fetched);
    apply_profiles(users, &known)
}

/// How long a person's directory record serves the snapshots of later connections.
const PROFILE_TTL: Duration = Duration::from_secs(30 * 60);

/// The directory records the snapshots of session `generation` fetched, with when (see
/// [`fill_profiles_cached`]).
#[derive(Default)]
struct ProfileCache {
    generation: u64,
    fetched: std::collections::HashMap<String, (std::time::Instant, mostik_core::UserProfile)>,
    /// People the directory answered about without a record (a lone one gets a 404): asked
    /// again only after [`PROFILE_TTL`], not at every reconnection.
    absent: std::collections::HashMap<String, std::time::Instant>,
}

/// The directory records of `guids`, a few requests at once, and the GUIDs the directory
/// answered about, with a record or without: a reply with `"status": "ok"`, or the 404 a request
/// for unknown people only gets. Any other failure answers nothing.
async fn fetch_profiles(web: &WebClient, guids: &[String]) -> (Vec<mostik_core::UserProfile>, Vec<String>) {
    let jobs = guids.chunks(convert::PROFILES_PER_REQUEST).map(|chunk| {
        let mut web = web.clone_light();
        async move { (chunk, web.rpc("get_users_data", json!({ "guids": chunk })).await) }
    });
    let mut profiles = Vec::new();
    let mut answered = Vec::new();
    for (chunk, result) in futures_util::future::join_all(jobs).await {
        match result {
            Ok(v) if v.get("status").and_then(Value::as_str) == Some("ok") => profiles.extend(convert::profiles(&v)),
            Ok(v) => {
                profiles.extend(convert::profiles(&v));
                continue;
            }
            Err(WebError::NotFound { .. }) => {}
            Err(_) => continue,
        }
        answered.extend(chunk.iter().cloned());
    }
    (profiles, answered)
}

/// Fills in what `users` lack from their directory records. Returns how many photos were filled in.
fn apply_profiles(users: &mut [User], profiles: &[mostik_core::UserProfile]) -> usize {
    let mut filled = 0;
    for p in profiles {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The first message of a sending goes as the user chose, the rest quietly and unmarked.
    #[test]
    fn only_the_first_message_of_a_sending_carries_its_options() {
        use mostik_ipc::cmd::SendOptions;
        use mostik_yandex::wire::Notify;
        let both = SendOptions {
            silent: true,
            important: true,
            follows: false,
        };
        assert_eq!(sending_options(both, true), (Notify::Muted, true));
        assert_eq!(sending_options(both, false), (Notify::Silent, false));
        assert_eq!(sending_options(SendOptions::default(), true), (Notify::Default, false));
        assert_eq!(sending_options(SendOptions::default(), false), (Notify::Silent, false));
    }

    /// A text too big comes back with an error status and an empty message info: the send fails
    /// with the known phrase, not with a missing message time. A sent message passes.
    #[test]
    fn a_push_refused_as_too_big_fails() {
        let too_big = json!({"Status": 13, "Details": "message too big: 100476", "MessageInfo": {}});
        assert_eq!(rejection(&too_big).as_deref(), Some(mostik_ipc::cmd::MESSAGE_TOO_LONG));
        let other = json!({"Status": 13, "Details": "no rights", "MessageInfo": {}});
        assert_eq!(
            rejection(&other).as_deref(),
            Some("Telemost rejected the request: no rights")
        );
        let sent = json!({"Status": 1, "MessageInfo": {"TimestampMcs": 1, "SeqNo": 2}});
        assert_eq!(rejection(&sent), None);
        assert!(rejection(&json!({"Status": 13})).is_some());
    }

    /// Two fetches of a new direct chat may end in any order: a later one that did not learn the
    /// name leaves the name the other one recorded.
    #[test]
    fn a_stand_in_title_does_not_replace_a_name() {
        let chat = Chat::new(
            ChatId("u0_u1".into()),
            ChatKind::Direct {
                peer: UserId("u1".into()),
            },
        );
        let mut sh = Shared::default();
        sh.note_chat(&chat, String::new());
        assert_eq!(sh.titles["u0_u1"], "");
        sh.note_chat(&chat, "Ann".into());
        sh.note_chat(&chat, mostik_core::UNKNOWN_USER.into());
        sh.note_chat(&chat, String::new());
        assert_eq!(sh.titles["u0_u1"], "Ann");
        sh.note_chat(&chat, "Anna".into());
        assert_eq!(sh.titles["u0_u1"], "Anna");
    }
}
