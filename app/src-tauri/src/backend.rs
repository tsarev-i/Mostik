//! Commands for the UI: connection, history, writes and live events.
//!
//! Connection diagnostics, in a debug build or with [`PROBE_ENV`] set: every connection writes
//! `probe.json`, and the first socket events go to `probe-events.jsonl` (in the app data dir).
//! They hold only structure (keys and types) and identifier patterns: no message text, names
//! or tokens.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use mostik_core::{
    CallNote, ChatFolder, ChatId, ChatKind, Message, NotifyLevel, Presence, Snapshot, Timestamp, User, UserId,
};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::{id_pattern, shape};
use mostik_yandex::identity;
use mostik_yandex::passport;
use mostik_yandex::session::CookieJar;
use mostik_yandex::web::{WebClient, WebError};
use mostik_yandex::wire::{self, HistoryRequest, PushRequest, request_id};
use mostik_yandex::xiva_client::{Incoming, XivaClient, XivaError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Mutex;

use crate::{call, login, store};

pub mod service;

/// UA of the embedded WebKitGTK; the unofficial client label is appended to it.
const WEBVIEW_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/60.5 Safari/605.1.15";

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
/// History page sizes. A page of 50 takes the server 0.5–1.5 s in a busy channel and 20 about
/// 0.1–0.3 s, so a chat opens with a short page; older pages and threads take more at once.
const FIRST_PAGE: u32 = 20;
const OLDER_PAGE: u32 = 40;
const THREAD_PAGE: u32 = 50;
/// Seconds a history page may take: twice that with the repeat, about what one wait used to be.
const HISTORY_TIMEOUT_S: u64 = 12;
/// Socket events written to `probe-events.jsonl` per run.
const MAX_DIAG_EVENTS: usize = 300;
/// Turns on the connection diagnostics (`probe*.json`, `probe-events.jsonl`) in a release build.
const PROBE_ENV: &str = "MOSTIK_PROBE";

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

/// In-memory attachment cache: showing an image again does not hit the network.
#[derive(Default)]
struct FileCache {
    items: std::collections::HashMap<String, (Arc<Vec<u8>>, String)>,
    order: std::collections::VecDeque<String>,
    bytes: usize,
}

const FILE_CACHE_LIMIT: usize = 64 * 1024 * 1024;

/// The largest attachment served to the UI (an image, whole or a preview): it is held in memory.
const SERVED_FILE_LIMIT: u64 = 50 * 1024 * 1024;

impl FileCache {
    fn get(&self, key: &str) -> Option<(Arc<Vec<u8>>, String)> {
        self.items.get(key).cloned()
    }

    fn put(&mut self, key: String, bytes: Arc<Vec<u8>>, kind: String) {
        if self.items.contains_key(&key) || bytes.len() > FILE_CACHE_LIMIT / 4 {
            return;
        }
        self.bytes += bytes.len();
        self.order.push_back(key.clone());
        self.items.insert(key, (bytes, kind));
        while self.bytes > FILE_CACHE_LIMIT {
            let Some(old) = self.order.pop_front() else { break };
            if let Some((b, _)) = self.items.remove(&old) {
                self.bytes -= b.len();
            }
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

/// UI settings the backend needs (from "Preferences" in the UI).
#[derive(Clone)]
pub struct Prefs {
    /// `all`, `direct` or `nothing`.
    pub notify: String,
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
            notify: "all".into(),
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
    /// Contents of the files pasted or dropped into the message field, by their [`KEPT`] path.
    kept_files: StdMutex<BTreeMap<String, Vec<u8>>>,
    live: Mutex<Option<Arc<Live>>>,
    session: Session,
    reconnecting: AtomicBool,
    heartbeat_running: AtomicBool,
    diag_events: Arc<AtomicUsize>,
    pub service: service::State,
}

/// Which sign-in the backend works for. Signing in and out starts a new generation, and work
/// begun for an older one (a reconnect, a cookie refresh, a push of its connection) is dropped
/// instead of signing the user back in or writing the old cookies back.
#[derive(Default)]
struct Session {
    generation: AtomicU64,
    /// Held while the password store is written or cleared, so that a save checked against the
    /// generation cannot land after the sign-out has deleted the session.
    store: StdMutex<()>,
}

impl Session {
    fn current(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn is_current(&self, generation: u64) -> bool {
        self.current() == generation
    }

    fn lock_store(&self) -> std::sync::MutexGuard<'_, ()> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts a new generation (a sign-in or a sign-out) and returns it.
    fn renew(&self) -> u64 {
        let _store = self.lock_store();
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Saves the cookies of session `generation`, unless it has ended.
    fn save(&self, generation: u64, jar: &CookieJar) -> Result<(), String> {
        let _store = self.lock_store();
        if !self.is_current(generation) {
            return Err("the session has ended".into());
        }
        store::save(jar)
    }

    /// Deletes the saved session, unless a newer one than `generation` has replaced it.
    fn delete(&self, generation: u64) -> Result<(), String> {
        let _store = self.lock_store();
        if !self.is_current(generation) {
            return Ok(());
        }
        store::delete()
    }
}

/// Makes `live` the connection, unless session `generation` has ended meanwhile.
async fn install(state: &AppState, generation: u64, live: Arc<Live>) -> bool {
    let mut slot = state.live.lock().await;
    if !state.session.is_current(generation) {
        return false;
    }
    *slot = Some(live);
    true
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

#[tauri::command]
pub async fn bootstrap(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Boot, String> {
    let generation = state.session.current();
    match store::load() {
        None => Ok(Boot::NeedLogin { reason: None }),
        Some(jar) => connect(&app, &state, jar, false, generation).await,
    }
}

#[tauri::command]
pub async fn login(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Boot, String> {
    let rx = login::open(&app)?;
    let jar = rx.await.map_err(|_| {
        text(
            &app,
            "Вход отменён: окно Яндекс ID закрыто.",
            "Sign-in cancelled: the Yandex ID window was closed.",
        )
        .to_owned()
    })?;
    let generation = state.session.renew();
    state.session.save(generation, &jar)?;
    connect(&app, &state, jar, true, generation).await
}

/// "Sign out": ends the Yandex ID session on the server, as the Telemost web client does, then
/// forgets it here: the password store entry, the connection, the call and what the account
/// left on disk. The app is signed out in any case; an error says what did not work out, and
/// the UI shows it on the login screen.
#[tauri::command]
pub async fn logout(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // From here on nothing of the old session is saved again or reconnected.
    let generation = state.session.renew();
    let live = state.live.lock().await.take();
    // It waits for the call browser to go: not on an async worker.
    let closing = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || call::shutdown(&closing)).await;
    crate::ring::forget(&app);
    let jar = live.as_ref().map(|l| l.web.jar().clone()).or_else(store::load);
    drop(live);
    let revoked = match jar {
        Some(jar) => passport::logout(&jar, &identity::user_agent(WEBVIEW_UA))
            .await
            .map_err(|e| {
                format!(
                    "{}: {e}",
                    text(
                        &app,
                        "Яндекс ID не завершил сессию, она может оставаться действующей",
                        "Yandex ID did not end the session, it may still be valid"
                    )
                )
            }),
        None => Ok(()),
    };
    let deleted = state.session.delete(generation).map_err(|e| {
        format!(
            "{}: {e}",
            text(
                &app,
                "Сессия осталась в хранилище паролей, при следующем запуске вход будет выполнен снова",
                "The session stays in the password store and the next start signs in again"
            )
        )
    });
    forget_local_data(&app, &state).await;
    // A session left in the password store comes back at the next start: that one matters most.
    deleted.and(revoked)
}

/// Removes what the signed-out account left on this computer: the diagnostic probes (they name
/// its cookies and chats), the custom emoji cache, the files allowed for sending, and the
/// cookies and caches of the main window. The UI clears its own `localStorage` keys.
async fn forget_local_data(app: &AppHandle, state: &AppState) {
    if let Ok(dir) = app.path().app_data_dir()
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("probe") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    service::forget_account(app).await;
    if let Ok(mut allowed) = state.allowed_files.lock() {
        allowed.clear();
    }
    if let Some(main) = app.get_webview_window("main") {
        crate::webview::forget_site_data(&main, crate::webview::SiteData::CookiesAndCaches);
    }
}

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
        for u in users.iter().filter(|u| u.has_name()) {
            sh.names.insert(u.id.0.clone(), u.display_name.clone());
        }
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
                for u in profiles.iter().filter(|u| u.has_name()) {
                    sh.names.insert(u.id.0.clone(), u.display_name.clone());
                }
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
async fn history_page(
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
    let resp = push(&live, msg).await?;
    let ts = resp
        .pointer("/MessageInfo/TimestampMcs")
        .and_then(convert::uint)
        .ok_or("Telemost did not return the message time")?;
    Ok(Message {
        chat_id: ChatId(chat_id),
        ts,
        author: live.me.clone(),
        text,
        thread_parent: thread,
        edited: false,
        important: false,
        reactions: vec![],
        thread: None,
        seq_no: resp.pointer("/MessageInfo/SeqNo").and_then(convert::uint).unwrap_or(0),
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
async fn original_plain(live: &Live, target: &str, ts: Timestamp) -> Result<Value, String> {
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

/// Serves an attachment for the `mostikfile://localhost/<id>?size=…` scheme.
pub async fn serve_file(app: &AppHandle, path: &str, query: Option<&str>) -> Result<(Vec<u8>, String), String> {
    let id = percent_decode(path.trim_start_matches('/'));
    if id.is_empty() || id.contains("..") {
        return Err("bad file identifier".into());
    }
    if let Some(emoji) = id.strip_prefix("emoji/") {
        return service::serve(app, emoji).await;
    }
    // A preview of a file the user picked (the image of a new custom emoji); no other files.
    if let Some(path) = id.strip_prefix("local/") {
        let state = app.state::<AppState>();
        if !state.allowed_files.lock().is_ok_and(|a| a.contains(path)) {
            return Err("not a picked file".into());
        }
        let bytes = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
        let kind = mime_guess::from_path(path)
            .first_or_octet_stream()
            .essence_str()
            .to_owned();
        return Ok((bytes, kind));
    }
    let size = query
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("size=")))
        .filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    let started = std::time::Instant::now();
    let live = live(&app.state::<AppState>()).await?;
    let (bytes, kind) = cached_file(&live, &id, size).await?;
    eprintln!(
        "file {} {size:?}: {} bytes, {kind}, {:?}",
        id_pattern(&id),
        bytes.len(),
        started.elapsed()
    );
    Ok((bytes.as_ref().clone(), kind))
}

async fn cached_file(live: &Live, id: &str, size: Option<&str>) -> Result<(Arc<Vec<u8>>, String), String> {
    let key = format!("{id}?{}", size.unwrap_or(""));
    let cached = || live.files.lock().ok().and_then(|c| c.get(&key));
    if let Some(hit) = cached() {
        return Ok(hit);
    }
    let lock = live
        .downloads
        .lock()
        .map_err(|e| e.to_string())?
        .entry(key.clone())
        .or_default()
        .clone();
    let _guard = lock.lock().await;
    // Someone else may have fetched it while we waited for the lock.
    if let Some(hit) = cached() {
        return Ok(hit);
    }
    let fetched = live
        .web
        .fetch_file(id, size, SERVED_FILE_LIMIT)
        .await
        .map_err(|e| e.to_string());
    if let Ok(mut d) = live.downloads.lock() {
        d.remove(&key);
    }
    let (bytes, kind) = fetched?;
    let bytes = Arc::new(bytes);
    if let Ok(mut c) = live.files.lock() {
        c.put(key, bytes.clone(), kind.clone());
    }
    Ok((bytes, kind))
}

/// How many image previews are fetched at the same time.
const PREFETCH_CONCURRENCY: usize = 6;

/// Fetches image previews in parallel while the UI renders the history.
fn prefetch_previews(live: Arc<Live>, messages: &[Message]) {
    let ids: Vec<String> = messages
        .iter()
        .flat_map(|m| m.attachments.iter())
        .filter(|a| a.kind == mostik_core::AttachmentKind::Image)
        .map(|a| a.file_id.clone())
        .collect();
    if ids.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        use futures_util::StreamExt;
        // Owned futures: a stream of closures over borrowed IDs trips the `Send` check of `spawn`.
        let jobs: Vec<_> = ids
            .into_iter()
            .map(|id| {
                let live = live.clone();
                async move {
                    let _ = cached_file(&live, &id, Some("middle")).await;
                }
            })
            .collect();
        futures_util::stream::iter(jobs)
            .buffer_unordered(PREFETCH_CONCURRENCY)
            .for_each(|_| async {})
            .await;
    });
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Downloads an attachment to "Downloads" and opens it. Returns the file path.
#[tauri::command]
pub async fn save_file(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    file_id: String,
    name: String,
) -> Result<String, String> {
    let live = live(&state).await?;
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let (path, file) = crate::downloads::create_new(&dir, &crate::downloads::safe_name(&name))
        .await
        .map_err(|e| format!("could not save the file: {e}"))?;
    download_to(&live, &file_id, file, &path).await?;
    crate::downloads::mark_from_internet(&path);
    let shown = path.display().to_string();
    open_external(&shown)?;
    Ok(shown)
}

/// Writes an attachment into `file`, just created at `path`, as it arrives, whatever its size;
/// a failed download leaves no file behind.
async fn download_to(
    live: &Live,
    file_id: &str,
    mut file: tokio::fs::File,
    path: &std::path::Path,
) -> Result<(), String> {
    let downloaded = live.web.download(file_id, &mut file).await;
    drop(file);
    if let Err(e) = downloaded {
        let _ = tokio::fs::remove_file(path).await;
        return Err(e.to_string());
    }
    Ok(())
}

/// "Edit" in your own profile: the name and photo are edited in Yandex ID.
#[tauri::command]
pub fn open_yandex_id() -> Result<(), String> {
    open_external("https://id.yandex.ru/personal")
}

/// A link from a message opens in the default browser. Only `http`, `https` and `mailto`:
/// the address comes from someone else's text, so files and other schemes must not open.
/// A Telemost meeting link opens in the call window, like a huddle link in Slack.
#[tauri::command]
pub async fn open_url(app: AppHandle, state: tauri::State<'_, AppState>, url: String) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    let allowed = ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p));
    if !allowed || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("this link cannot be opened".into());
    }
    if call::is_meeting_url(&url)
        && let Ok(live) = live(&state).await
    {
        let info = call::CallInfo {
            chat_id: None,
            label: text(&app, "Встреча в Телемосте", "Telemost meeting").to_owned(),
        };
        return call::open(&app, &url, info, live.web.jar()).await;
    }
    open_external(&url)
}

/// Opens a link or a file in the program the system has for it.
#[cfg(not(target_os = "windows"))]
pub(crate) fn open_external(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    command.arg(url);
    // With the AppImage's libraries xdg-open and the browser it starts would crash on startup.
    #[cfg(target_os = "linux")]
    crate::appimage::outside(&mut command);
    command
        .spawn()
        .map(drop)
        .map_err(|e| format!("could not open the browser: {e}"))
}

/// Opens a link or a file in the program the system has for it. ShellExecuteW, not
/// `cmd /C start`: cmd would treat `&`, `|` and `^` in a link as its own syntax, cutting the
/// link short or running what follows `&` as a command.
#[cfg(target_os = "windows")]
pub(crate) fn open_external(url: &str) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};

    let target = HSTRING::from(url);
    // SAFETY: both strings are NUL-terminated UTF-16 that live until the call returns.
    let result = unsafe { ShellExecuteW(None, w!("open"), &target, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    // Values above 32 mean success; the rest are error codes.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(format!("could not open the browser: error {}", result.0 as usize))
    }
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

fn err_json(e: impl ToString) -> Value {
    json!({ "error": e.to_string() })
}

/// Whether the connection diagnostics are on: in a debug build, or with `MOSTIK_PROBE=1`.
pub(crate) fn probing() -> bool {
    static ON: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| cfg!(debug_assertions) || std::env::var(PROBE_ENV).is_ok_and(|v| v == "1"));
    *ON
}

/// Writes diagnostics file `name` into the app data dir, when they are on ([`probing`]).
pub(crate) fn write_probe(app: &AppHandle, name: &str, probe: &Value) {
    if !probing() {
        return;
    }
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(name), serde_json::to_string_pretty(probe).unwrap_or_default());
    }
}

/// `signed_in`: the session has just come from the login window, so a rejection means the
/// sign-in did not go through rather than an expired session. `generation`: see [`Session`].
async fn connect(
    app: &AppHandle,
    state: &AppState,
    jar: CookieJar,
    signed_in: bool,
    generation: u64,
) -> Result<Boot, String> {
    match connect_probed(app, state, jar, generation).await {
        Ok((snapshot, live)) => {
            let live = Arc::new(live);
            if !install(state, generation, live.clone()).await {
                return Ok(Boot::NeedLogin { reason: None });
            }
            start_heartbeat(app);
            service::start(app.clone(), live.clone());
            Ok(Boot::Ready {
                snapshot: Box::new(snapshot),
            })
        }
        Err(ConnectError::Unauthorized) => {
            if let Err(e) = state.session.delete(generation) {
                eprintln!("{e}");
            }
            let reason = if signed_in {
                text(
                    app,
                    "Телемост не принял вход через Яндекс ID. Попробуйте войти ещё раз.",
                    "Telemost did not accept the Yandex ID sign-in. Please try again.",
                )
            } else {
                text(
                    app,
                    "Сессия Яндекс ID недействительна, войдите снова.",
                    "Your Yandex ID session has expired. Please sign in again.",
                )
            };
            Ok(Boot::NeedLogin {
                reason: Some(reason.into()),
            })
        }
        Err(ConnectError::Other(e)) => Err(e),
    }
}

#[derive(Serialize)]
pub struct Threads {
    threads: Vec<mostik_core::FollowedThread>,
    users: Vec<User>,
}

/// Threads the user follows, with unread counts, like Slack's "Threads". The UI asks once per
/// connection and then keeps the list current from the pushes. The thread ID format goes to
/// the diagnostics.
#[tauri::command]
pub async fn followed_threads(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Threads, String> {
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
    let (threads, users) = remember_threads(&live, &v.map_err(|e| e.to_string())?);
    Ok(Threads { threads, users })
}

/// Parses the followed threads and records them as the ones whose replies notify.
fn remember_threads(live: &Live, v: &Value) -> (Vec<mostik_core::FollowedThread>, Vec<User>) {
    let (mut threads, users) = convert::followed_threads(v, Some(&live.me));
    if let Ok(mut sh) = live.shared.lock() {
        threads.retain(|t| !sh.service.contains(&t.parent.chat_id.0));
        sh.followed = threads
            .iter()
            .map(|t| (t.parent.chat_id.0.clone(), t.parent.ts))
            .collect();
    }
    (threads, users)
}

/// Fetches the followed threads again a moment after a reply in a thread the list did not
/// have (several replies wait for one fetch): Telemost may have subscribed the user to it,
/// for example for a mention. The UI gets the new list, and the waiting replies notify if
/// their thread is followed now.
async fn refresh_threads(app: AppHandle, shared: Arc<StdMutex<Shared>>) {
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let fetched = match live(&app.state::<AppState>()).await {
        Ok(live) if Arc::ptr_eq(&live.shared, &shared) => live
            .xiva
            .call("history", &wire::thread_list(0), 30)
            .await
            .map(|v| remember_threads(&live, &v))
            .map_err(|e| eprintln!("threads: {e}"))
            .ok(),
        _ => None,
    };
    let waiting = {
        let Ok(mut sh) = shared.lock() else { return };
        let waiting = std::mem::take(&mut sh.thread_wait);
        if fetched.is_some() {
            for (m, _) in &waiting {
                if let Some(key) = thread_key(m)
                    && !sh.followed.contains(&key)
                {
                    sh.not_followed.insert(key);
                }
            }
        }
        waiting
    };
    let Some((threads, users)) = fetched else { return };
    eprintln!(
        "threads fetched again for {} replies: {} followed",
        waiting.len(),
        threads.len()
    );
    let _ = app.emit(EVENT, &LiveEvent::Threads { threads, users });
    for (m, author) in waiting {
        let followed = shared
            .lock()
            .is_ok_and(|sh| thread_key(&m).is_some_and(|k| sh.followed.contains(&k)));
        if followed {
            notify(&app, &shared, &m, author.as_ref());
        }
    }
}

/// `(chat, parent)` of a thread reply.
fn thread_key(m: &Message) -> Option<(String, Timestamp)> {
    m.thread_parent.map(|p| (m.chat_id.0.clone(), p))
}

/// Whether a reply can notify now: it is in a followed thread or is not a reply. A reply in a
/// thread of unknown standing waits for the list to be fetched again (see [`refresh_threads`]).
fn reply_notifies_now(app: &AppHandle, shared: &Arc<StdMutex<Shared>>, m: &Message, author: Option<&User>) -> bool {
    let Some(key) = thread_key(m) else { return true };
    let Ok(mut sh) = shared.lock() else { return false };
    if sh.me.as_ref() == Some(&m.author) {
        // Telemost subscribes the user to a thread they reply in.
        sh.not_followed.remove(&key);
        sh.followed.insert(key);
        return false;
    }
    if sh.followed.contains(&key) {
        return true;
    }
    if m.edited || !sh.ready || (sh.not_followed.contains(&key) && !mentions_me(&sh, m)) {
        return false;
    }
    let first = sh.thread_wait.is_empty();
    sh.thread_wait.push((m.clone(), author.cloned()));
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
    chat_id: String,
    parent: Timestamp,
    ts: Timestamp,
    seq_no: u64,
) -> Result<(), String> {
    let live = live(&state).await?;
    let thread = convert::thread_chat_id(&chat_id, parent).ok_or("Threads are not supported in this chat")?;
    push(&live, wire::seen_marker(&thread, ts, seq_no)).await.map(|_| ())
}

enum ConnectError {
    Unauthorized,
    Other(String),
}

impl From<WebError> for ConnectError {
    fn from(e: WebError) -> Self {
        match e {
            WebError::Unauthorized => ConnectError::Unauthorized,
            other => ConnectError::Other(other.to_string()),
        }
    }
}

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
fn handle_incoming(
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
            if let Some(name) = sh.titles.remove(id) {
                sh.channels.retain(|c| mostik_core::group_label(c) != name);
            }
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
        // Known from now on even if the fetch failed: its messages are not held again.
        let title = match &fetched {
            Some((chat, users)) => match &chat.kind {
                ChatKind::Direct { peer } => users
                    .iter()
                    .find(|u| &u.id == peer)
                    .map(|u| u.display_name.clone())
                    .or_else(|| sh.names.get(&peer.0).cloned())
                    .unwrap_or_default(),
                _ => mostik_core::group_label(&chat.name),
            },
            None => String::new(),
        };
        sh.titles.insert(chat_id.clone(), title);
        if let Some((chat, users)) = fetched {
            for u in users.iter().filter(|u| u.has_name()) {
                sh.names.insert(u.id.0.clone(), u.display_name.clone());
            }
            if chat.kind == (ChatKind::Group { public: true }) && !sh.channels.contains(&chat.name) {
                sh.channels.push(chat.name.clone());
            }
            let section = convert::section_of(&chat).to_owned();
            let _ = app.emit(EVENT, &LiveEvent::ChatAdded { chat, section, users });
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
fn unnamed_mentions(messages: &[Message], known: &[User]) -> Vec<User> {
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
    User {
        id,
        display_name: String::new(),
        title: None,
        presence: mostik_core::Presence::Offline,
        status: None,
        availability: Default::default(),
        is_robot: false,
        avatar: None,
    }
}

/// Names of the people a new message mentions: the ones Telemost sent with it are recorded,
/// the rest are asked from the directory and handed to the UI as [`LiveEvent::Users`].
fn learn_mentions(app: &AppHandle, shared: &Arc<StdMutex<Shared>>, m: &Message, mentioned: &[User]) {
    let unknown: Vec<User> = {
        let Ok(mut sh) = shared.lock() else { return };
        for u in mentioned.iter().filter(|u| u.has_name()) {
            sh.names.insert(u.id.0.clone(), u.display_name.clone());
        }
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
            for u in &users {
                sh.names.insert(u.id.0.clone(), u.display_name.clone());
            }
        }
        let _ = app.emit(EVENT, &LiveEvent::Users { users });
    });
}

async fn fetch_chat(live: &Live, chat_id: &str) -> Option<(mostik_core::Chat, Vec<User>)> {
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

/// Runs `f` with the people, public chats and ids `@` can point at.
fn with_names<R>(sh: &Shared, f: impl FnOnce(&mostik_core::markup::Mentionable) -> R) -> R {
    let people: Vec<&str> = sh.names.values().map(String::as_str).collect();
    let channels: Vec<&str> = sh.channels.iter().map(String::as_str).collect();
    let ids: Vec<(&str, &str)> = sh.names.iter().map(|(i, n)| (i.as_str(), n.as_str())).collect();
    f(&mostik_core::markup::Mentionable {
        people: &people,
        channels: &channels,
        ids: &ids,
    })
}

/// Whether message `m` mentions the user: by id, by name, through a public chat or `@all`.
fn mentions_me(sh: &Shared, m: &Message) -> bool {
    sh.me.as_ref().is_some_and(|me| {
        let me = mostik_core::markup::Me {
            id: &me.0,
            name: sh.names.get(&me.0).map_or("", String::as_str),
        };
        with_names(sh, |names| mostik_core::markup::mentions_me(&m.text, names, me))
    })
}

/// Whether notifications are paused now.
fn paused(sh: &Shared) -> bool {
    sh.paused.is_some_and(|p| p.active_at(js_now_mcs()))
}

/// Whether a call ringing in `m`'s chat may ring: not in "Do not disturb", not in a chat that
/// notifies about nothing, not with notifications off. Unlike messages, a call rings even when
/// the window is in focus, as in Slack.
fn may_ring(app: &AppHandle, shared: &StdMutex<Shared>, m: &Message) -> bool {
    let Ok(sh) = shared.lock() else { return false };
    if !sh.ready
        || sh.me.as_ref() == Some(&m.author)
        || paused(&sh)
        || sh.muted.get(&m.chat_id.0) == Some(&NotifyLevel::Nothing)
    {
        return false;
    }
    let state = app.state::<AppState>();
    state.prefs.lock().is_ok_and(|p| p.notify != "nothing")
}

fn notify(app: &AppHandle, shared: &StdMutex<Shared>, m: &Message, author: Option<&User>) {
    let Ok(mut sh) = shared.lock() else { return };
    // A thread reply lives in the thread chat: its time and number mean nothing in the parent chat.
    // A ringing call is not a message in the chat: the read marker cannot point at it.
    if m.thread_parent.is_none() && !m.is_ringing() {
        let entry = sh.last.entry(m.chat_id.0.clone()).or_default();
        if m.ts > entry.0 {
            *entry = (m.ts, m.seq_no.max(entry.1));
        }
    }
    // Only the start of a call notifies; "the call ended" and changes of the chat are just lines in it.
    let ended = matches!(m.call, Some(mostik_core::CallNote::Ended { .. }));
    if !sh.ready || sh.me.as_ref() == Some(&m.author) || m.edited || ended || m.system.is_some() {
        return;
    }
    // A muted chat notifies about mentions only, or about nothing.
    match sh.muted.get(&m.chat_id.0) {
        Some(NotifyLevel::Nothing) => return,
        Some(NotifyLevel::Mentions) if !mentions_me(&sh, m) => return,
        _ => {}
    }
    if paused(&sh) {
        return;
    }
    let prefs = app
        .state::<AppState>()
        .prefs
        .lock()
        .map(|p| p.clone())
        .unwrap_or_default();
    let direct = !m.chat_id.0.contains('/');
    // Only "direct and mentions" needs to know about mentions.
    let mentioned = || mentions_me(&sh, m);
    match prefs.notify.as_str() {
        "nothing" => return,
        "direct" if !direct && !mentioned() => return,
        _ => {}
    }
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if focused {
        return;
    }
    let who = author
        .map(|a| a.display_name.clone())
        .or_else(|| sh.names.get(&m.author.0).cloned())
        .unwrap_or_else(|| text(app, "Новое сообщение", "New message").into());
    let title = match sh.titles.get(&m.chat_id.0) {
        Some(chat) if chat != &who => format!("{who} · {chat}"),
        _ => who,
    };
    let body: String = if m.call.is_some() {
        text(app, "📞 Начался созвон", "📞 A call started").to_owned()
    } else if prefs.preview {
        with_names(&sh, |names| mostik_core::markup::plain(&m.text, names))
            .chars()
            .take(200)
            .collect()
    } else {
        text(app, "Новое сообщение", "New message").to_owned()
    };
    let body = if body.is_empty() && !m.attachments.is_empty() {
        text(app, "📎 Вложение", "📎 Attachment").to_owned()
    } else {
        body
    };
    // A click on the notification opens this message (see `open_from_notification`).
    let mut builder = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .extra("chat", &m.chat_id.0)
        .extra("ts", m.ts);
    if let Some(icon) = notification_icon(app) {
        builder = builder.icon(icon);
    }
    if let Some(parent) = m.thread_parent {
        builder = builder.extra("thread", parent);
    }
    if let Err(e) = builder.show() {
        eprintln!("notification not shown: {e}");
    }
    crate::sound::play(&prefs.sound);
}

/// Plays a notification sound: the preview in the preferences.
#[tauri::command]
pub fn play_sound(id: String) {
    crate::sound::play(&id);
}

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
pub async fn open_dm(state: tauri::State<'_, AppState>, guid: String) -> Result<mostik_core::Chat, String> {
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
    let chat = mostik_core::Chat {
        id: ChatId(id.to_owned()),
        kind: convert::chat_kind(id, Some(&live.me), false),
        name: String::new(),
        description: None,
        member_count: 2,
        unread: 0,
        mentions: 0,
        muted: false,
        mute_mentions: false,
        last_ts: 0,
        peer_seen_ts: 0,
        pinned: None,
        organizations: vec![],
    };
    note_chat(&live, &chat);
    Ok(chat)
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

/// Group details from `get_chats_info` (`{chat_ids: [...]}`).
async fn chat_info_raw(live: &Live, chat_id: &str) -> Result<Value, String> {
    let mut web = live.web.clone_light();
    let v = web
        .rpc("get_chats_info", json!({ "chat_ids": [chat_id] }))
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
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

#[tauri::command]
pub async fn chat_details(state: tauri::State<'_, AppState>, chat_id: String) -> Result<ChatDetails, String> {
    let live = live(&state).await?;
    let info = chat_info_raw(&live, &chat_id).await?;
    let text = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    Ok(ChatDetails {
        name: text("name"),
        description: text("description"),
        role: text("role"),
        member_count: info.get("members_count").and_then(convert::uint).unwrap_or(0),
    })
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
) -> Result<mostik_core::Chat, String> {
    let live = live(&state).await?;
    let mut web = live.web.clone_light();
    let mut all = vec![live.me.0.clone()];
    all.extend(members.into_iter().filter(|m| m != &live.me.0));
    let v = web
        .rpc(
            "create_chat",
            json!({ "name": name, "members": all, "channel": false, "public": public }),
        )
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
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
        let r = web
            .rpc("set_chat_info", json!({ "chat_id": id, "version": version, "name": name, "description": description, "public": public }))
            .await
            .map_err(|e| e.to_string())?;
        rpc_ok(&r)?;
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
    let chat = mostik_core::Chat {
        id: ChatId(id),
        kind: ChatKind::Group { public: server_public },
        name: server_name,
        description: (!server_desc.trim().is_empty()).then_some(server_desc),
        member_count: info
            .get("members_count")
            .and_then(Value::as_u64)
            .map_or(all.len() as u32, |n| n as u32),
        unread: 0,
        mentions: 0,
        muted: false,
        mute_mentions: false,
        last_ts: js_now_mcs(),
        peer_seen_ts: 0,
        pinned: None,
        organizations: vec![],
    };
    note_chat(&live, &chat);
    Ok(chat)
}

fn js_now_mcs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn chat_members(
    state: tauri::State<'_, AppState>,
    chat_id: String,
) -> Result<Vec<mostik_core::ChatMember>, String> {
    let live = live(&state).await?;
    let mut web = live.web.clone_light();
    let v = web
        .rpc("get_chat_members", json!({ "chat_id": chat_id, "limit": 200 }))
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
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
    let mut web = live.web.clone_light();
    let v = web
        .rpc(
            "change_chat_members",
            json!({ "chat_id": chat_id, "version": version, "users": users, "role": "member" }),
        )
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)
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
    let mut web = live.web.clone_light();
    let v = web
        .rpc("set_chat_info", json!({ "chat_id": chat_id, "version": version, "name": name, "description": description, "public": public }))
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
    // Return what the server actually saved: that is what the UI shows.
    let info = chat_info_raw(&live, &chat_id).await?;
    let text = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    Ok(ChatDetails {
        name: text("name"),
        description: text("description"),
        role: text("role"),
        member_count: info.get("members_count").and_then(convert::uint).unwrap_or(0),
    })
}

#[tauri::command]
pub async fn leave_chat(state: tauri::State<'_, AppState>, chat_id: String) -> Result<(), String> {
    let live = live(&state).await?;
    let version = chat_version(&live, &chat_id).await?;
    let mut web = live.web.clone_light();
    let v = web
        .rpc("leave", json!({ "chat_id": chat_id, "version": version }))
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
    if let Ok(mut sh) = live.shared.lock()
        && let Some(name) = sh.titles.remove(&chat_id)
    {
        // Titles keep a group's emoji as `⚠️ Test`, the channel list its raw name `⚠️|Test`.
        sh.channels.retain(|c| mostik_core::group_label(c) != name);
    }
    Ok(())
}

/// A chat joined or created after the start: its title for notifications, and its name for
/// `@channel` mentions if it is public.
fn note_chat(live: &Live, chat: &mostik_core::Chat) {
    let Ok(mut sh) = live.shared.lock() else { return };
    sh.titles
        .insert(chat.id.0.clone(), mostik_core::group_label(&chat.name));
    let public = chat.kind == ChatKind::Group { public: true };
    if public && !chat.name.is_empty() && !sh.channels.contains(&chat.name) {
        sh.channels.push(chat.name.clone());
    }
}

/// Changes a synced setting (`set_bucket`): reads the current value and version,
/// applies `change` and writes it back whole. Retries on a version conflict.
async fn update_bucket(live: &Live, name: &str, change: impl Fn(&mut Value)) -> Result<Value, String> {
    let mut web = live.web.clone_light();
    for _ in 0..3 {
        let v = web
            .rpc("get_buckets", json!({ "version": 0 }))
            .await
            .map_err(|e| e.to_string())?;
        rpc_ok(&v)?;
        let bucket = v
            .pointer("/data/buckets")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|b| b["bucket_name"] == name).cloned())
            .unwrap_or(Value::Null);
        let mut value = bucket
            .get("bucket_value")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let version = bucket.get("version").and_then(Value::as_u64).unwrap_or(0);
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
    let id = chat_id.clone();
    let (mute, mute_mentions) = level.flags();
    update_bucket(&live, "chat_mutings", move |v| {
        v[&id] = json!({ "mute": mute, "mute_mentions": mute_mentions })
    })
    .await?;
    if let Ok(mut sh) = live.shared.lock() {
        if level == NotifyLevel::All {
            sh.muted.remove(&chat_id);
        } else {
            sh.muted.insert(chat_id, level);
        }
    }
    Ok(())
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
    ts: Timestamp,
    seq_no: u64,
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
    let resp = push(&live, json!({ "Plain": plain })).await?;
    Ok(Sent {
        ts: resp
            .pointer("/MessageInfo/TimestampMcs")
            .and_then(convert::uint)
            .ok_or("Telemost did not return the message time")?,
        seq_no: resp.pointer("/MessageInfo/SeqNo").and_then(convert::uint).unwrap_or(0),
    })
}

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
        let mut web = live.web.clone_light();
        let v = web
            .rpc(
                "create_meeting",
                json!({ "chat_id": chat_id, "supported_features": [] }),
            )
            .await
            .map_err(|e| e.to_string())?;
        rpc_ok(&v)?;
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

/// Group from an HTTP RPC response (`get_recommended_chats`, `activate_chat`).
fn rpc_chat(v: &Value, me: &UserId) -> Option<mostik_core::Chat> {
    let id = v.get("chat_id")?.as_str()?;
    let public = v.get("public").and_then(Value::as_bool).unwrap_or(false)
        || v.get("invite_hash")
            .and_then(Value::as_str)
            .is_some_and(|h| !h.is_empty());
    Some(mostik_core::Chat {
        id: ChatId(id.to_owned()),
        kind: convert::chat_kind(id, Some(me), public),
        name: v.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
        description: v
            .get("description")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
            .map(str::to_owned),
        member_count: v.get("members_count").and_then(Value::as_u64).unwrap_or(0) as u32,
        unread: 0,
        mentions: 0,
        muted: false,
        mute_mentions: false,
        last_ts: 0,
        peer_seen_ts: 0,
        pinned: None,
        organizations: v
            .get("organization_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(convert::uint)
            .collect(),
    })
}

/// "Browse channels": public groups recommended by Telemost.
#[tauri::command]
pub async fn recommended_chats(state: tauri::State<'_, AppState>) -> Result<Vec<mostik_core::Chat>, String> {
    let live = live(&state).await?;
    let mut web = live.web.clone_light();
    let v = web
        .rpc("get_recommended_chats", json!({ "limit": 50 }))
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
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
pub async fn join_chat(state: tauri::State<'_, AppState>, chat_id: String) -> Result<mostik_core::Chat, String> {
    let live = live(&state).await?;
    let mut web = live.web.clone_light();
    let v = web
        .rpc("activate_chat", json!({ "chat_id": chat_id }))
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
    let chat = v
        .pointer("/data/chat")
        .and_then(|c| rpc_chat(c, &live.me))
        .ok_or_else(|| "Telemost did not return the chat".to_owned())?;
    note_chat(&live, &chat);
    Ok(chat)
}

/// "+" in the composer: the system file picker.
#[tauri::command]
pub async fn pick_files(app: AppHandle) -> Result<Vec<PendingFile>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_files(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = rx.await.map_err(|e| e.to_string())?.unwrap_or_default();
    let paths: Vec<std::path::PathBuf> = picked.into_iter().filter_map(|f| f.into_path().ok()).collect();
    Ok(allow_files(&app, &paths))
}

/// Path prefix of the files kept by [`keep_file`]: they have no path on disk.
const KEPT: &str = "kept:";

/// A file pasted or dropped into the message field: its contents come in the request body, its
/// name (URI-encoded) in the `x-file-name` header. It is kept in memory, so that a pasted
/// screenshot leaves nothing on disk, and is sent like a picked file.
#[tauri::command]
pub async fn keep_file(
    state: tauri::State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<PendingFile, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("no file contents".into());
    };
    let name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .unwrap_or_default();
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.trim().is_empty() && !n.chars().any(char::is_control))
        .unwrap_or("file");
    let path = format!("{KEPT}{}/{name}", request_id());
    let file = PendingFile {
        path: path.clone(),
        name: name.to_owned(),
        size: bytes.len() as u64,
        image: mime_guess::from_path(name)
            .first()
            .is_some_and(|m| m.type_() == "image"),
    };
    state
        .kept_files
        .lock()
        .map_err(|e| e.to_string())?
        .insert(path.clone(), bytes.clone());
    if let Ok(mut allowed) = state.allowed_files.lock() {
        allowed.insert(path);
    }
    Ok(file)
}

/// Uploads the files and sends them: images as one gallery with a caption, the rest one by one.
/// For non-images the caption goes as a separate text message before them.
#[tauri::command]
pub async fn send_files(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    thread: Option<Timestamp>,
    paths: Vec<String>,
    text: String,
) -> Result<Vec<Message>, String> {
    let live = live(&state).await?;
    let allowed = state.allowed_files.lock().map(|a| a.clone()).unwrap_or_default();
    if let Some(bad) = paths.iter().find(|p| !allowed.contains(*p)) {
        return Err(format!("the file was not picked by the user: {bad}"));
    }
    let target = target_chat(&chat_id, thread)?;
    let gallery_id = request_id();
    let kept: Vec<Option<Vec<u8>>> = {
        let all = state.kept_files.lock().map_err(|e| e.to_string())?;
        paths.iter().map(|p| all.get(p).cloned()).collect()
    };
    // Files are read and uploaded a few at a time; `buffered` keeps their order for the gallery.
    let uploads: Vec<_> = paths
        .iter()
        .zip(kept)
        .map(|(path, kept)| upload_one(live.web.clone_light(), target.clone(), gallery_id.clone(), path, kept))
        .collect();
    let uploaded: Vec<Uploaded> = {
        use futures_util::{StreamExt, TryStreamExt};
        futures_util::stream::iter(uploads)
            .buffered(UPLOAD_CONCURRENCY)
            .try_collect()
            .await?
    };
    let (mut images, mut files) = (Vec::new(), Vec::new());
    for u in uploaded {
        match u {
            Uploaded::Image(item) => images.push(item),
            Uploaded::File(payload, info) => files.push((payload, info)),
        }
    }
    let mut sent = Vec::new();
    let text = text.trim().to_owned();
    if !text.is_empty() && images.is_empty() {
        sent.push(
            send_plain(
                &live,
                &chat_id,
                thread,
                wire::plain_text(&target, &request_id(), &text),
                &text,
            )
            .await?,
        );
    }
    if !images.is_empty() {
        let msg = json!({ "Plain": { "ChatId": target, "PayloadId": gallery_id, "Gallery": { "Items": images, "Text": text } } });
        sent.push(send_plain(&live, &chat_id, thread, msg, &text).await?);
    }
    for (payload, info) in files {
        let msg = json!({ "Plain": { "ChatId": target, "PayloadId": payload, "MiscFile": { "FileInfo": info } } });
        sent.push(send_plain(&live, &chat_id, thread, msg, "").await?);
    }
    if let Ok(mut kept) = state.kept_files.lock() {
        for p in &paths {
            kept.remove(p);
        }
    }
    Ok(sent)
}

/// How many files are uploaded at the same time.
const UPLOAD_CONCURRENCY: usize = 3;

/// An uploaded file: a gallery item for an image, or a payload ID and `FileInfo` for any other file.
enum Uploaded {
    Image(Value),
    File(String, Value),
}

/// Reads a file without blocking the runtime (unless its contents are `kept` in memory) and
/// uploads it to the Messenger storage.
async fn upload_one(
    mut web: WebClient,
    target: String,
    gallery_id: String,
    path: &str,
    kept: Option<Vec<u8>>,
) -> Result<Uploaded, String> {
    let p = std::path::Path::new(path);
    let bytes = match kept {
        Some(bytes) => bytes,
        None => tokio::fs::read(p).await.map_err(|e| format!("{path}: {e}"))?,
    };
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let mime = mime_guess::from_path(p).first_or_octet_stream();
    let size = bytes.len();
    let dims = (mime.type_() == "image")
        .then(|| imagesize::blob_size(&bytes).ok())
        .flatten();
    let payload = if dims.is_some() { gallery_id } else { request_id() };
    let up = web
        .upload_file(&target, &payload, &name, mime.essence_str(), bytes)
        .await
        .map_err(|e| e.to_string())?;
    let id = up
        .get("file_id")
        .and_then(Value::as_str)
        .ok_or("Telemost did not return file_id")?
        .to_owned();
    let info = json!({ "Id2": id, "Name": name, "Size": size });
    Ok(match dims {
        Some(d) => Uploaded::Image(json!({ "Image": { "Width": d.width, "Height": d.height, "FileInfo": info } })),
        None => Uploaded::File(payload, info),
    })
}

/// Sends a ready `ClientMessage` and returns the feed message (with its attachments).
async fn send_plain(
    live: &Live,
    chat_id: &str,
    thread: Option<Timestamp>,
    msg: Value,
    text: &str,
) -> Result<Message, String> {
    let resp = push(live, msg.clone()).await?;
    let ts = resp
        .pointer("/MessageInfo/TimestampMcs")
        .and_then(convert::uint)
        .ok_or("Telemost did not return the message time")?;
    let fake = json!({ "ClientMessage": msg, "ServerMessageInfo": { "Timestamp": ts, "From": { "Guid": live.me.0 } } });
    let mut m = match convert::from_push(&fake, Some(&live.me)) {
        Some(LiveEvent::Upsert { message, .. }) => *message,
        _ => return Err("could not parse the sent message".into()),
    };
    m.chat_id = ChatId(chat_id.to_owned());
    m.thread_parent = thread;
    if !text.is_empty() {
        m.text = text.to_owned();
    }
    Ok(m)
}

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
fn start_heartbeat(app: &AppHandle) {
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

/// Icon of the notifications, as a file path. Without one the notification plugin gives the name
/// of the executable as the icon, which no icon theme has, and Plasma draws a broken image next
/// to the text. The app icon is written to the cache once per run.
fn notification_icon(app: &AppHandle) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        PATH.get_or_init(|| {
            const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");
            let dir = app.path().app_cache_dir().ok()?;
            let path = dir.join("notification-icon.png");
            if std::fs::read(&path).ok().as_deref() != Some(ICON) {
                std::fs::create_dir_all(&dir).ok()?;
                std::fs::write(&path, ICON).ok()?;
            }
            path.to_str().map(str::to_owned)
        })
        .clone()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        None
    }
}

/// "Show an example" in the notification preferences.
#[tauri::command]
pub fn test_notification(app: AppHandle) -> Result<(), String> {
    let mut builder = app
        .notification()
        .builder()
        .title(text(&app, "Мостик", "Mostik"))
        .body(text(
            &app,
            "Так выглядит уведомление о новом сообщении.",
            "This is what a new message notification looks like.",
        ));
    if let Some(icon) = notification_icon(&app) {
        builder = builder.icon(icon);
    }
    builder.show().map_err(|e| {
        format!(
            "{}: {e}",
            text(
                &app,
                "система не показала уведомление",
                "the system didn't show the notification"
            )
        )
    })
}

/// Main window title: the UI language and the unread count, as in Slack.
#[tauri::command]
pub fn set_window_title(app: AppHandle, title: String) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window.set_title(&title).map_err(|e| e.to_string())
}

/// UI zoom from "Preferences → Accessibility" and Ctrl+= / Ctrl+-.
#[tauri::command]
pub fn set_zoom(app: AppHandle, scale: f64) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window.set_zoom(scale.clamp(0.5, 2.0)).map_err(|e| e.to_string())
}

/// Settings from the UI that the backend needs: notifications, previews and language.
#[tauri::command]
pub fn set_preferences(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    notify: String,
    preview: bool,
    lang: String,
    sound: Option<String>,
) {
    let english = lang == "en";
    if let Ok(mut p) = state.prefs.lock() {
        *p = Prefs {
            notify,
            preview,
            english,
            sound: sound.unwrap_or_else(|| p.sound.clone()),
        };
    }
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.open.set_text(text(&app, "Открыть Мостик", "Open Mostik"));
        let _ = items.quit.set_text(text(&app, "Выйти", "Quit"));
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(text(
            &app,
            "Мостик | неофициальный клиент Телемост",
            "Mostik | unofficial Telemost client",
        )));
    }
}

/// Reconnects session `generation` after its connection closed. One loop at a time: the closes
/// that come while it runs are its own business.
async fn reconnect_loop(app: AppHandle, mut generation: u64) {
    let state = app.state::<AppState>();
    loop {
        if state.reconnecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let reconnected = reconnect(&app, &state, generation).await;
        state.reconnecting.store(false, Ordering::SeqCst);
        // A connection that closed while the flag turned its own reconnection away would
        // otherwise stay dead: the new one closing at once, or that of a sign-in that came
        // while this loop still worked for the session before.
        let dead = state.live.lock().await.as_ref().is_some_and(|l| l.xiva.is_closed());
        let ended = !state.session.is_current(generation);
        if !(dead && (reconnected || ended)) {
            return;
        }
        generation = state.session.current();
        eprintln!("reconnect: the connection closed while reconnecting");
        let _ = app.emit(EVENT, &LiveEvent::Connection { online: false });
    }
}

/// Connects session `generation` again and again, with growing pauses, until a connection is
/// up (`true`), there is no session to connect with or it has ended (`false`).
async fn reconnect(app: &AppHandle, state: &AppState, generation: u64) -> bool {
    let mut attempt = 0;
    loop {
        tokio::time::sleep(reconnect_pause(attempt, random())).await;
        attempt = attempt.saturating_add(1);
        // Signed out meanwhile: nothing to restore.
        if !state.session.is_current(generation) {
            return false;
        }
        let Some(jar) = store::load() else { return false };
        match connect_probed(app, state, jar, generation).await {
            // Its close went to a reconnection that this loop turned away: try again.
            Ok((_, live)) if live.xiva.is_closed() => eprintln!("reconnect: the new connection closed at once"),
            Ok((snapshot, live)) => {
                let live = Arc::new(live);
                if !install(state, generation, live.clone()).await {
                    return false;
                }
                // Changes to the service chat made while offline.
                service::start(app.clone(), live);
                let _ = app.emit(EVENT, &LiveEvent::Connection { online: true });
                let _ = app.emit(SNAPSHOT_EVENT, &snapshot);
                return true;
            }
            Err(ConnectError::Unauthorized) => {
                if !state.session.is_current(generation) {
                    return false;
                }
                if let Err(e) = state.session.delete(generation) {
                    eprintln!("{e}");
                }
                call::close(app);
                *state.live.lock().await = None;
                let reason = text(
                    app,
                    "Сессия Яндекс ID недействительна, войдите снова.",
                    "Your Yandex ID session has expired. Please sign in again.",
                );
                let _ = app.emit(SIGNED_OUT_EVENT, reason);
                return false;
            }
            Err(ConnectError::Other(e)) => eprintln!("reconnect failed: {e}"),
        }
    }
}

/// The pause before reconnection attempt `attempt` (from 0): 2, 4, 8 … 60 s, each stretched by
/// up to a half at random (`random` is any number), so that clients cut off together (a
/// Telemost restart, a network change at an office) do not come back all at once.
fn reconnect_pause(attempt: u32, random: u64) -> Duration {
    let base = 2_000u64.saturating_mul(1 << attempt.min(5)).min(60_000);
    Duration::from_millis(base + random % (base / 2 + 1))
}

/// A random number, for the jitter of [`reconnect_pause`]: std's hasher keys are random.
fn random() -> u64 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one(std::time::Instant::now())
}

/// Chat count and chat ID patterns of a `history` response, for the probe.
fn id_patterns(v: &Value) -> Value {
    let chats = v.get("Chats").and_then(Value::as_array);
    let patterns: BTreeSet<String> = chats
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("ChatId").and_then(Value::as_str))
        .map(id_pattern)
        .collect();
    json!({ "chats": chats.map_or(0, Vec::len), "id_patterns": patterns })
}

/// `whoami` over the socket, tried a few times: an answer without `UserInfo` counts as a failure.
/// Every attempt goes to the probe; `Null` if none succeeded.
async fn whoami(xiva: &XivaClient, probe: &mut Value) -> Value {
    let mut attempts = Vec::new();
    for _ in 0..3 {
        let who = xiva.call("whoami", &json!({ "RequestId": request_id() }), 10).await;
        attempts.push(who.as_ref().map(shape).unwrap_or_else(err_json));
        match who {
            Ok(v) if v.pointer("/UserInfo/Guid").and_then(Value::as_str).is_some() => {
                probe["ws_whoami"] = json!(attempts);
                return v;
            }
            Err(XivaError::Closed) => break,
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    probe["ws_whoami"] = json!(attempts);
    Value::Null
}

/// [`connect_inner`], with what it found out written to `probe.json` (see [`probing`]).
async fn connect_probed(
    app: &AppHandle,
    state: &AppState,
    jar: CookieJar,
    generation: u64,
) -> Result<(Snapshot, Live), ConnectError> {
    let mut probe = json!({ "client": identity::ua_suffix() });
    let result = connect_inner(app, state, jar, generation, &mut probe).await;
    write_probe(app, "probe.json", &probe);
    result
}

async fn connect_inner(
    app: &AppHandle,
    state: &AppState,
    jar: CookieJar,
    generation: u64,
    probe: &mut Value,
) -> Result<(Snapshot, Live), ConnectError> {
    let ua = identity::user_agent(WEBVIEW_UA);
    probe["cookies"] = json!(
        jar.cookies
            .iter()
            .map(|c| format!("{}@{}", c.name, c.domain))
            .collect::<Vec<_>>()
    );
    let saved = jar.clone();
    let mut web = WebClient::new(jar, ua.clone())?;

    let page = web.load_page().await;
    probe["page"] = match &page {
        Ok(p) => json!({ "puid_len": p.puid.len(), "service_id": p.service_id.is_some(), "logged_in": p.logged_in }),
        Err(e) => err_json(e),
    };
    probe["page"]["hops"] = json!(web.page_hops);
    let page = page?;
    // The page refreshed some cookies on its redirects: keep them for the next start.
    if web.jar() != &saved {
        probe["page"]["jar_saved"] = json!(state.session.save(generation, web.jar()).is_ok());
    }

    let shared: Arc<StdMutex<Shared>> = Arc::default();
    let (handler_app, handler_shared, diag) = (app.clone(), shared.clone(), state.diag_events.clone());
    let xiva = XivaClient::connect(web.jar(), &ua, &page.puid, move |incoming| {
        // The connection of a session that has ended (signed out) has nothing more to say.
        if handler_app.state::<AppState>().session.is_current(generation) {
            handle_incoming(&handler_app, &handler_shared, &diag, generation, incoming)
        }
    })
    .await
    .map_err(|e| {
        probe["xiva"] = err_json(&e);
        ConnectError::Other(format!("Could not connect to Telemost: {e}"))
    })?;
    probe["xiva"] = json!({ "connected": true });

    // Xiva may ignore requests sent before it has subscribed the socket, so wait for that first.
    probe["subscription"] = json!(xiva.wait_subscription(Duration::from_secs(5)).await.is_some());
    let who = whoami(&xiva, probe).await;

    let list = xiva.call("history", &wire::chat_list(0), 40).await;
    probe["history"] = list.as_ref().map(id_patterns).unwrap_or_else(err_json);
    let list = list.map_err(|e| ConnectError::Other(format!("Telemost did not return the chat list: {e}")))?;

    // Without `whoami` every direct chat would show its first member, often yourself: take the
    // GUID from the chat IDs and the name, photo and organization from the directory instead.
    let (who, me_id) = match who.pointer("/UserInfo/Guid").and_then(Value::as_str).map(str::to_owned) {
        Some(guid) => (who, UserId(guid)),
        None => match convert::own_guid(&list) {
            Some(me) => {
                let profile = web.rpc("get_users_data", json!({ "guids": [me.0] })).await;
                probe["me_fallback"] = profile.as_ref().map(shape).unwrap_or_else(err_json);
                let profile = profile
                    .ok()
                    .and_then(|v| convert::profiles(&v).into_iter().find(|p| p.id == me));
                (convert::whoami_from_profile(&me, profile.as_ref()), me)
            }
            None => {
                probe["me_fallback"] = json!("unknown");
                (Value::Null, UserId("me".into()))
            }
        },
    };
    if me_id.0 != "me"
        && let Ok(mut sh) = shared.lock()
    {
        // Known before the rest of the snapshot, so pushes arriving meanwhile recognise our own messages.
        sh.me = Some(me_id.clone());
    }
    let me_name = who
        .pointer("/UserInfo/DisplayName")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let organization = who
        .pointer("/UserInfo/OrganizationInfos/0/OrganizationName")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .unwrap_or("Telemost")
        .to_owned();

    let mut data = convert::from_history(&list, Some(&me_id));
    let service_chats = if me_id.0 == "me" {
        vec![]
    } else {
        service::detect(app, &mut web, &list, &me_id).await
    };
    probe["service_chats"] = json!(service_chats.len());
    // The chat with oneself, as in Slack: create it if it does not exist yet (the call is idempotent).
    let notes_id = convert::notes_chat_id(&me_id);
    if me_id.0 != "me" && !data.chats.iter().any(|c| c.id == notes_id) {
        match web.rpc("create_private_chat", json!({ "guid": me_id.0 })).await {
            Ok(v) if v.get("status").and_then(Value::as_str) == Some("ok") => {
                data.chats.push(convert::notes_chat(&me_id));
                probe["notes"] = json!("created");
            }
            Ok(v) => probe["notes"] = json!({ "code": v.pointer("/data/code") }),
            Err(e) => probe["notes"] = err_json(e),
        }
    } else {
        probe["notes"] = json!("exists");
    }
    let last = std::mem::take(&mut data.last);
    let me = User {
        id: me_id.clone(),
        display_name: me_name.unwrap_or_else(|| "You".into()),
        title: None,
        presence: Presence::Online,
        status: None,
        availability: Default::default(),
        is_robot: false,
        avatar: who
            .pointer("/UserInfo/AvatarId")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty())
            .map(str::to_owned),
    };
    let mut snapshot = convert::snapshot(me, organization, data);
    snapshot.organization_avatar = who
        .pointer("/UserInfo/OrganizationInfos/0/AvatarId")
        .and_then(Value::as_str)
        .filter(|a| !a.is_empty())
        .map(str::to_owned);
    snapshot.organizations = who
        .pointer("/UserInfo/OrganizationInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| {
            Some(mostik_core::Organization {
                id: o.get("OrganizationId").and_then(convert::uint)?,
                name: o
                    .get("OrganizationName")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                avatar: o
                    .get("AvatarId")
                    .and_then(Value::as_str)
                    .filter(|a| !a.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect();
    let filled = fill_profiles(&web, &mut snapshot.users).await;
    probe["profiles_filled"] = json!(filled);
    match web.rpc("get_buckets", json!({ "version": 0 })).await {
        Ok(b) => {
            convert::apply_buckets(&mut snapshot, &b);
            probe["buckets"] = json!(b.get("status"));
        }
        Err(e) => probe["buckets"] = err_json(e),
    }
    let service_chats = service::hide(&mut snapshot, service_chats);
    if let Ok(mut sh) = shared.lock() {
        sh.me = Some(me_id.clone());
        sh.paused = snapshot.paused;
        sh.service = service_chats.iter().map(|s| s.id.clone()).collect();
        sh.service_chats = service_chats;
        sh.orgs = snapshot.organizations.iter().map(|o| o.id).collect();
        // Pushes that arrived while connecting may already be newer than the history.
        for (chat, (ts, seq)) in last {
            let entry = sh.last.entry(chat).or_default();
            if ts > entry.0 {
                *entry = (ts, seq.max(entry.1));
            }
        }
        for ch in &snapshot.chats {
            if ch.muted {
                sh.muted.insert(ch.id.0.clone(), ch.notify_level());
            }
            let title = match ch.kind {
                ChatKind::Group { .. } | ChatKind::Broadcast => mostik_core::group_label(&ch.name),
                _ => mostik_core::chat_title(&snapshot, ch),
            };
            sh.titles.insert(ch.id.0.clone(), title);
        }
        for u in snapshot.users.iter().filter(|u| u.has_name()) {
            sh.names.insert(u.id.0.clone(), u.display_name.clone());
        }
        sh.channels = snapshot.mention_channels();
        sh.ready = true;
    }
    probe["snapshot"] = json!({
        "chats": snapshot.chats.len(),
        "users": snapshot.users.len(),
        "messages": snapshot.messages.len(),
        "direct": snapshot.chats.iter().filter(|c| matches!(c.kind, ChatKind::Direct { .. })).count(),
        "me_known": me_id.0 != "me",
        "with_avatar": snapshot.users.iter().filter(|u| u.avatar.is_some()).count(),
        "org_avatar": snapshot.organization_avatar.is_some(),
    });
    Ok((
        snapshot,
        Live {
            files: StdMutex::default(),
            downloads: StdMutex::default(),
            web,
            xiva,
            me: me_id,
            user_agent: ua,
            service_id: page.service_id,
            shared,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{percent_decode, reconnect_pause};
    use std::time::Duration;

    #[test]
    fn reconnect_pauses_grow_with_jitter() {
        let ms = |attempt, random| reconnect_pause(attempt, random).as_millis();
        assert_eq!(
            [0, 1, 2, 3, 4, 5, 40].map(|a| ms(a, 0)),
            [2000, 4000, 8000, 16000, 32000, 60000, 60000]
        );
        // Up to a half more, at random.
        assert_eq!(ms(0, 1000), 3000);
        assert_eq!(ms(0, 1001), 2000);
        assert_eq!(ms(5, u64::MAX), 60000 + u128::from(u64::MAX % 30001));
        assert!(reconnect_pause(40, u64::MAX) <= Duration::from_secs(90));
    }

    #[test]
    fn decodes_file_ids() {
        assert_eq!(percent_decode("disk%2Fabc-1"), "disk/abc-1");
        assert_eq!(percent_decode("%D1%84%D0%B0%D0%B9%D0%BB"), "файл");
        assert_eq!(percent_decode("bad%2"), "bad%2");
        assert_eq!(percent_decode("plain"), "plain");
    }
}
