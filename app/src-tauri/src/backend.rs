//! Commands for the UI: connection, history, writes and live events.
//!
//! On connect `probe.json` is written, and the first socket events go to `probe-events.jsonl`
//! (in the app data dir). They hold only structure (keys and types) and identifier
//! patterns: no message text, names or tokens.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use mostik_core::{ChatId, ChatKind, Message, Presence, Snapshot, Timestamp, User, UserId};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::{id_pattern, shape};
use mostik_yandex::identity;
use mostik_yandex::session::CookieJar;
use mostik_yandex::web::{WebClient, WebError};
use mostik_yandex::wire::{self, HistoryRequest, PushRequest, request_id};
use mostik_yandex::xiva_client::{Incoming, XivaClient, XivaError};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Mutex;

use crate::{call, login, store};

/// UA of the embedded WebKitGTK; the unofficial client label is appended to it.
const WEBVIEW_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/60.5 Safari/605.1.15";

/// Live event for the UI.
pub const EVENT: &str = "mostik://event";
/// Fresh snapshot after a reconnect.
pub const SNAPSHOT_EVENT: &str = "mostik://snapshot";
/// The session is no longer valid: the UI shows the login screen with this reason.
pub const SIGNED_OUT_EVENT: &str = "mostik://signed_out";
const PAGE_SIZE: u32 = 50;
const MAX_DIAG_EVENTS: usize = 300;

/// Result of `bootstrap` and `login`: either the login screen or a ready snapshot.
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Boot {
    NeedLogin { reason: Option<String> },
    Ready { snapshot: Snapshot },
}

/// State updated from live events.
#[derive(Default)]
struct Shared {
    me: Option<UserId>,
    /// Own availability: no notifications in "Do not disturb" mode.
    my_availability: mostik_core::Availability,
    /// The last `(Timestamp, SeqNo)` per chat, for read markers.
    last: BTreeMap<String, (Timestamp, u64)>,
    muted: BTreeSet<String>,
    titles: BTreeMap<String, String>,
    names: BTreeMap<String, String>,
    /// Public group chats the user is in: `@<name>` mentions them too.
    channels: Vec<String>,
    /// Set once the snapshot is built: until then `muted`, `titles` and `names` are incomplete,
    /// so no notifications are shown.
    ready: bool,
}

/// In-memory attachment cache: showing an image again does not hit the network.
#[derive(Default)]
struct FileCache {
    items: std::collections::HashMap<String, (Arc<Vec<u8>>, String)>,
    order: std::collections::VecDeque<String>,
    bytes: usize,
}

const FILE_CACHE_LIMIT: usize = 64 * 1024 * 1024;

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
}

impl Default for Prefs {
    fn default() -> Self {
        let english = std::env::var("LANG")
            .map(|l| !l.to_lowercase().starts_with("ru"))
            .unwrap_or(false);
        Self {
            notify: "all".into(),
            preview: true,
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
    live: Mutex<Option<Arc<Live>>>,
    reconnecting: AtomicBool,
    heartbeat_running: AtomicBool,
    diag_events: Arc<AtomicUsize>,
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
    match store::load() {
        None => Ok(Boot::NeedLogin { reason: None }),
        Some(jar) => connect(&app, &state, jar).await,
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
    store::save(&jar)?;
    connect(&app, &state, jar).await
}

#[tauri::command]
pub async fn logout(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    store::delete();
    call::close(&app);
    *state.live.lock().await = None;
    Ok(())
}

/// History page: the latest messages before `before` (exclusive).
#[tauri::command]
pub async fn history(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    before: Option<Timestamp>,
    thread: Option<Timestamp>,
) -> Result<Vec<Message>, String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    let body = live
        .xiva
        .call("history", &HistoryRequest::page(&target, PAGE_SIZE, before), 20)
        .await
        .map_err(|e| e.to_string())?;
    let data = convert::from_history(&body, Some(&live.me));
    if before.is_none()
        && thread.is_none()
        && let Ok(mut sh) = live.shared.lock()
    {
        sh.last.extend(data.last);
    }
    eprintln!("history {}: {} messages", id_pattern(&chat_id), data.messages.len());
    prefetch_previews(live.clone(), &data.messages);
    Ok(data.messages)
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

#[tauri::command]
pub async fn mark_read(state: tauri::State<'_, AppState>, chat_id: String) -> Result<(), String> {
    let live = live(&state).await?;
    let last = live.shared.lock().ok().and_then(|sh| sh.last.get(&chat_id).copied());
    let Some((ts, seq)) = last.filter(|(ts, seq)| *ts > 0 && *seq > 0) else {
        return Ok(());
    };
    push(&live, wire::seen_marker(&chat_id, ts, seq)).await.map(|_| ())
}

#[tauri::command]
pub async fn send_message(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    text: String,
    thread: Option<Timestamp>,
) -> Result<Message, String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    let resp = push(&live, wire::plain_text(&target, &request_id(), &text, &[])).await?;
    let ts = resp
        .pointer("/MessageInfo/TimestampMcs")
        .and_then(Value::as_u64)
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
        seq_no: resp.pointer("/MessageInfo/SeqNo").and_then(Value::as_u64).unwrap_or(0),
        attachments: vec![],
        forwarded: vec![],
    })
}

#[tauri::command]
pub async fn edit_message(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Timestamp,
    text: String,
    thread: Option<Timestamp>,
) -> Result<(), String> {
    let live = live(&state).await?;
    let target = target_chat(&chat_id, thread)?;
    push(&live, wire::edit_text(&target, ts, &request_id(), &text))
        .await
        .map(|_| ())
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
    let fetched = live.web.fetch_file(id, size).await.map_err(|e| e.to_string());
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
    let (bytes, _) = live.web.fetch_file(&file_id, None).await.map_err(|e| e.to_string())?;
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let safe: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let safe = if safe.trim().is_empty() || safe.starts_with('.') {
        format!("file{safe}")
    } else {
        safe
    };
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
        _ => (safe.clone(), String::new()),
    };
    let mut path = dir.join(&safe);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("could not save the file: {e}"))?;
    let shown = path.display().to_string();
    open_external(&shown)?;
    Ok(shown)
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

pub(crate) fn open_external(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let (program, args) = ("xdg-open", vec![url]);
    #[cfg(target_os = "macos")]
    let (program, args) = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let (program, args) = ("cmd", vec!["/C", "start", "", url]);
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open the browser: {e}"))
}

fn err_json(e: impl ToString) -> Value {
    json!({ "error": e.to_string() })
}

fn write_probe(app: &AppHandle, name: &str, probe: &Value) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(name), serde_json::to_string_pretty(probe).unwrap_or_default());
    }
}

async fn connect(app: &AppHandle, state: &AppState, jar: CookieJar) -> Result<Boot, String> {
    let mut probe = json!({ "client": identity::ua_suffix() });
    let result = connect_inner(app, state, jar, &mut probe).await;
    write_probe(app, "probe.json", &probe);
    match result {
        Ok((snapshot, live)) => {
            let live = Arc::new(live);
            *state.live.lock().await = Some(live.clone());
            start_heartbeat(app);
            probe_threads(app.clone(), live);
            Ok(Boot::Ready { snapshot })
        }
        Err(ConnectError::Unauthorized) => {
            store::delete();
            Ok(Boot::NeedLogin {
                reason: Some(
                    text(
                        app,
                        "Сессия Яндекс ID недействительна, войдите снова.",
                        "Your Yandex ID session has expired. Please sign in again.",
                    )
                    .into(),
                ),
            })
        }
        Err(ConnectError::Other(e)) => Err(e),
    }
}

/// Diagnostics of the thread ID format, in the background so that it never delays the start.
fn probe_threads(app: AppHandle, live: Arc<Live>) {
    tauri::async_runtime::spawn(async move {
        let probe = match live.xiva.call("history", &wire::thread_list(0), 30).await {
            Ok(v) => {
                let mut entry = id_patterns(&v);
                entry["shape"] = shape(&v);
                entry
            }
            Err(e) => err_json(e),
        };
        write_probe(&app, "probe-threads.json", &probe);
    });
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

/// Socket event: diagnostics, `Shared` update, UI event, notification.
fn handle_incoming(app: &AppHandle, shared: &StdMutex<Shared>, diag: &AtomicUsize, incoming: Incoming) {
    match incoming {
        Incoming::Push { event, body } => {
            if diag.fetch_add(1, Ordering::SeqCst) < MAX_DIAG_EVENTS
                && let Ok(dir) = app.path().app_data_dir()
                && let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join("probe-events.jsonl"))
            {
                let _ = writeln!(f, "{}", json!({ "event": event, "shape": shape(&body) }));
            }
            let me = shared.lock().ok().and_then(|sh| sh.me.clone());
            let Some(ev) = convert::from_push(&body, me.as_ref()) else {
                return;
            };
            if let LiveEvent::Status(p) = &ev
                && me.as_ref() == Some(&p.user)
                && let Ok(mut sh) = shared.lock()
            {
                sh.my_availability = p.availability;
            }
            if let LiveEvent::Upsert { message, author } = &ev {
                notify(app, shared, message, author.as_ref());
            }
            let _ = app.emit(EVENT, &ev);
        }
        Incoming::Text(_) => {}
        Incoming::Closed => {
            let _ = app.emit(EVENT, &LiveEvent::Connection { online: false });
            let app = app.clone();
            tauri::async_runtime::spawn(async move { reconnect_loop(app).await });
        }
    }
}

fn notify(app: &AppHandle, shared: &StdMutex<Shared>, m: &Message, author: Option<&User>) {
    let Ok(mut sh) = shared.lock() else { return };
    // A thread reply lives in the thread chat: its time and number mean nothing in the parent chat.
    if m.thread_parent.is_none() {
        let entry = sh.last.entry(m.chat_id.0.clone()).or_default();
        if m.ts > entry.0 {
            *entry = (m.ts, m.seq_no.max(entry.1));
        }
    }
    if !sh.ready || sh.me.as_ref() == Some(&m.author) || sh.muted.contains(&m.chat_id.0) || m.edited {
        return;
    }
    if sh.my_availability == mostik_core::Availability::DoNotDisturb {
        return;
    }
    let prefs = app
        .state::<AppState>()
        .prefs
        .lock()
        .map(|p| p.clone())
        .unwrap_or_default();
    let direct = !m.chat_id.0.contains('/');
    let mentioned = sh
        .me
        .as_ref()
        .and_then(|me| sh.names.get(&me.0))
        .is_some_and(|me_name| {
            let who = mostik_core::markup::MentionNames {
                people: sh.names.values().cloned().collect(),
                channels: sh.channels.clone(),
            };
            who.with(|who| mostik_core::markup::mentions_me(&m.text, who, me_name))
        });
    match prefs.notify.as_str() {
        "nothing" => return,
        "direct" if !direct && !mentioned => return,
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
    let body: String = if prefs.preview {
        mostik_core::markup::plain(&m.text).chars().take(200).collect()
    } else {
        text(app, "Новое сообщение", "New message").to_owned()
    };
    let body = if body.is_empty() && !m.attachments.is_empty() {
        text(app, "📎 Вложение", "📎 Attachment").to_owned()
    } else {
        body
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("notification not shown: {e}");
    }
    // The UI plays the sound (the one chosen in the preferences).
    let _ = app.emit("mostik://sound", ());
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
    Ok(mostik_core::Chat {
        id: ChatId(id.to_owned()),
        kind: convert::chat_kind(id, Some(&live.me), false),
        name: String::new(),
        description: None,
        member_count: 2,
        unread: 0,
        mentions: 0,
        muted: false,
        last_ts: 0,
        pinned: None,
        organizations: vec![],
    })
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
        member_count: info.get("members_count").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// People search for invitations. Without `chat_id` the search is relative to the notes chat.
#[tauri::command]
pub async fn search_people(
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
        .map_err(|e| e.to_string())?;
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
    Ok(mostik_core::Chat {
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
        last_ts: js_now_mcs(),
        pinned: None,
        organizations: vec![],
    })
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
        member_count: info.get("members_count").and_then(Value::as_u64).unwrap_or(0),
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
    rpc_ok(&v)
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

/// "Mute" for a chat (the `chat_mutings` bucket).
#[tauri::command]
pub async fn set_muted(state: tauri::State<'_, AppState>, chat_id: String, muted: bool) -> Result<(), String> {
    let live = live(&state).await?;
    let id = chat_id.clone();
    update_bucket(&live, "chat_mutings", move |v| {
        v[&id] = json!({ "mute": muted, "mute_mentions": false })
    })
    .await?;
    if let Ok(mut sh) = live.shared.lock() {
        if muted {
            sh.muted.insert(chat_id);
        } else {
            sh.muted.remove(&chat_id);
        }
    }
    Ok(())
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
            .and_then(Value::as_u64)
            .ok_or("Telemost did not return the message time")?,
        seq_no: resp.pointer("/MessageInfo/SeqNo").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// A call from a chat, like a Slack huddle: the meeting bound to the chat (`create_meeting`) opens
/// in the call window (`call.rs`). `meeting_id` is the meeting link itself. `label` is what the
/// window title and the sidebar show: `#dev` or the person's name.
#[tauri::command]
pub async fn start_call(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    chat_id: String,
    label: String,
) -> Result<(), String> {
    let live = live(&state).await?;
    let info = call::CallInfo {
        chat_id: Some(chat_id.clone()),
        label,
    };
    // `create_meeting` runs only when no call is open: it announces the call in the chat.
    let meeting = || async {
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
        Ok(if id.starts_with("https://") {
            id.to_owned()
        } else {
            format!("https://telemost.360.yandex.ru/j/{id}")
        })
    };
    call::open_chat_call(&app, info, live.web.jar(), meeting).await
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
        last_ts: 0,
        pinned: None,
        organizations: v
            .get("organization_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
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
    v.pointer("/data/chat")
        .and_then(|c| rpc_chat(c, &live.me))
        .ok_or_else(|| "Telemost did not return the chat".to_owned())
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
    // Files are read and uploaded a few at a time; `buffered` keeps their order for the gallery.
    let uploads: Vec<_> = paths
        .into_iter()
        .map(|path| upload_one(live.web.clone_light(), target.clone(), gallery_id.clone(), path))
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
                wire::plain_text(&target, &request_id(), &text, &[]),
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
    Ok(sent)
}

/// How many files are uploaded at the same time.
const UPLOAD_CONCURRENCY: usize = 3;

/// An uploaded file: a gallery item for an image, or a payload ID and `FileInfo` for any other file.
enum Uploaded {
    Image(Value),
    File(String, Value),
}

/// Reads a file without blocking the runtime and uploads it to the Messenger storage.
async fn upload_one(mut web: WebClient, target: String, gallery_id: String, path: String) -> Result<Uploaded, String> {
    let p = std::path::Path::new(&path);
    let bytes = tokio::fs::read(p).await.map_err(|e| format!("{path}: {e}"))?;
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
        .and_then(Value::as_u64)
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
    if let Some(mine) = out.get(&live.me.0)
        && let Ok(mut sh) = live.shared.lock()
    {
        sh.my_availability = mine.availability;
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
    let mut status = json!({ "Availability": availability });
    if !emoji.trim().is_empty() || !text.trim().is_empty() {
        status["CustomStatus"] = json!({ "Emoji": emoji.trim(), "Text": text.trim() });
    }
    if let Some(d) = duration.filter(|d| *d > 0) {
        status["Duration"] = json!(d);
    }
    push(&live, json!({ "UserStatus": status })).await?;
    if let Ok(mut sh) = live.shared.lock() {
        sh.my_availability = mostik_core::Availability::from_code(availability);
    }
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

/// "Show an example" in the notification preferences.
#[tauri::command]
pub fn test_notification(app: AppHandle) -> Result<(), String> {
    app.notification()
        .builder()
        .title(text(&app, "Мостик", "Mostik"))
        .body(text(
            &app,
            "Так выглядит уведомление о новом сообщении.",
            "This is what a new message notification looks like.",
        ))
        .show()
        .map_err(|e| {
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
pub fn set_preferences(app: AppHandle, state: tauri::State<'_, AppState>, notify: String, preview: bool, lang: String) {
    let english = lang == "en";
    if let Ok(mut p) = state.prefs.lock() {
        *p = Prefs {
            notify,
            preview,
            english,
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

/// Reconnects with pauses of 2, 4, 8 … 60 seconds.
async fn reconnect_loop(app: AppHandle) {
    let state = app.state::<AppState>();
    if state.reconnecting.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut delay = 2;
    loop {
        tokio::time::sleep(Duration::from_secs(delay)).await;
        let Some(jar) = store::load() else { break };
        let mut probe = json!({});
        match connect_inner(&app, &state, jar, &mut probe).await {
            Ok((snapshot, live)) => {
                *state.live.lock().await = Some(Arc::new(live));
                let _ = app.emit(EVENT, &LiveEvent::Connection { online: true });
                let _ = app.emit(SNAPSHOT_EVENT, &snapshot);
                break;
            }
            Err(ConnectError::Unauthorized) => {
                store::delete();
                call::close(&app);
                *state.live.lock().await = None;
                let reason = text(
                    &app,
                    "Сессия Яндекс ID недействительна, войдите снова.",
                    "Your Yandex ID session has expired. Please sign in again.",
                );
                let _ = app.emit(SIGNED_OUT_EVENT, reason);
                break;
            }
            Err(ConnectError::Other(e)) => eprintln!("reconnect failed: {e}"),
        }
        delay = (delay * 2).min(60);
    }
    state.reconnecting.store(false, Ordering::SeqCst);
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

async fn connect_inner(
    app: &AppHandle,
    state: &AppState,
    jar: CookieJar,
    probe: &mut Value,
) -> Result<(Snapshot, Live), ConnectError> {
    let ua = identity::user_agent(WEBVIEW_UA);
    probe["cookies"] = json!(
        jar.cookies
            .iter()
            .map(|c| format!("{}@{}", c.name, c.domain))
            .collect::<Vec<_>>()
    );
    let mut web = WebClient::new(jar, ua.clone())?;

    let page = web.load_page().await;
    probe["page"] = match &page {
        Ok(p) => json!({ "puid_len": p.puid.len(), "service_id": p.service_id.is_some(), "logged_in": p.logged_in }),
        Err(e) => err_json(e),
    };
    let page = page?;

    let shared: Arc<StdMutex<Shared>> = Arc::default();
    let (handler_app, handler_shared, diag) = (app.clone(), shared.clone(), state.diag_events.clone());
    let xiva = XivaClient::connect(web.jar(), &ua, &page.puid, move |incoming| {
        handle_incoming(&handler_app, &handler_shared, &diag, incoming)
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
                id: o.get("OrganizationId")?.as_u64()?,
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
    // Photos and job titles: the history does not always carry `AvatarId`, so fill them in from the directory.
    let guids: Vec<String> = snapshot.users.iter().map(|u| u.id.0.clone()).collect();
    let mut filled = 0;
    let jobs = guids.chunks(100).map(|chunk| {
        let mut web = web.clone_light();
        async move { web.rpc("get_users_data", json!({ "guids": chunk })).await }
    });
    for v in futures_util::future::join_all(jobs).await.into_iter().flatten() {
        for p in convert::profiles(&v) {
            if let Some(u) = snapshot.users.iter_mut().find(|u| u.id == p.id) {
                if u.avatar.is_none() && p.avatar.is_some() {
                    u.avatar = p.avatar.clone();
                    filled += 1;
                }
                if u.title.is_none() {
                    u.title = p.position.clone();
                }
                if !p.display_name.is_empty() && (u.display_name == u.id.0 || u.display_name.is_empty()) {
                    u.display_name = p.display_name.clone();
                }
            }
        }
    }
    probe["profiles_filled"] = json!(filled);
    match web.rpc("get_buckets", json!({ "version": 0 })).await {
        Ok(b) => {
            convert::apply_buckets(&mut snapshot, &b);
            probe["buckets"] = json!(b.get("status"));
        }
        Err(e) => probe["buckets"] = err_json(e),
    }
    if let Ok(mut sh) = shared.lock() {
        sh.me = Some(me_id.clone());
        // Pushes that arrived while connecting may already be newer than the history.
        for (chat, (ts, seq)) in last {
            let entry = sh.last.entry(chat).or_default();
            if ts > entry.0 {
                *entry = (ts, seq.max(entry.1));
            }
        }
        for ch in &snapshot.chats {
            if ch.muted {
                sh.muted.insert(ch.id.0.clone());
            }
            sh.titles
                .insert(ch.id.0.clone(), mostik_core::chat_title(&snapshot, ch));
        }
        for u in &snapshot.users {
            sh.names.insert(u.id.0.clone(), u.display_name.clone());
        }
        sh.channels = snapshot.mention_names().channels;
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
    use super::percent_decode;

    #[test]
    fn decodes_file_ids() {
        assert_eq!(percent_decode("disk%2Fabc-1"), "disk/abc-1");
        assert_eq!(percent_decode("%D1%84%D0%B0%D0%B9%D0%BB"), "файл");
        assert_eq!(percent_decode("bad%2"), "bad%2");
        assert_eq!(percent_decode("plain"), "plain");
    }
}
