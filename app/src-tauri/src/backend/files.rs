//! Files: attachments served to the UI from a memory cache and saved to "Downloads", links and
//! files opened outside the app, and the files the user picks, pastes and sends.

use std::collections::BTreeMap;
use std::sync::Arc;

use mostik_core::{ChatId, Message, Timestamp, UserId};
use mostik_ipc::cmd::{self, Args, FileArgs, Out, PathArgs, SendFilesArgs, UrlArgs};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::id_pattern;
use mostik_yandex::web::{WebClient, percent_decode};
use mostik_yandex::wire::{self, request_id};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use super::messages::sent_from_reply;
use super::{AppState, Live, PendingFile, allow_files, live, push, service, target_chat, text};
use crate::call;

/// In-memory attachment cache: showing an image again does not hit the network.
#[derive(Default)]
pub(super) struct FileCache {
    items: std::collections::HashMap<String, (Arc<Vec<u8>>, String)>,
    order: std::collections::VecDeque<String>,
    bytes: usize,
}

const FILE_CACHE_LIMIT: usize = 64 * 1024 * 1024;

/// The largest attachment served to the UI (an image, whole or a preview): it is held in memory.
const SERVED_FILE_LIMIT: u64 = 50 * 1024 * 1024;

/// The largest attachment saved to "Downloads".
const SAVED_FILE_LIMIT: u64 = 4 * 1024 * 1024 * 1024;

/// How many attachments are saved at the same time; one more is refused until one of them ends.
const SAVES_AT_ONCE: usize = 4;

/// The attachments of the messages the UI got, by file id: their names and sizes as the messages
/// give them. Only these are saved, under these names: a script in the window cannot have any
/// file saved under any name, and a download cannot run much past the size the message says.
/// They belong to one account: a late answer for an account signed out of does not mix in.
#[derive(Default)]
pub(super) struct KnownFiles {
    owner: Option<UserId>,
    files: std::collections::HashMap<String, KnownFile>,
}

#[derive(Default)]
struct KnownFile {
    /// The names the file has in messages, and where it was saved under each.
    names: BTreeMap<String, Option<std::path::PathBuf>>,
    size: u64,
}

/// What a save of a known file may do.
#[derive(Debug, PartialEq)]
struct SaveRule {
    /// How many bytes the download may take.
    limit: u64,
    /// Where it was saved already under that name.
    saved: Option<std::path::PathBuf>,
}

impl KnownFiles {
    /// Records the attachments of `messages` of account `me`, forwarded ones too.
    pub(super) fn note<'a>(&mut self, me: &UserId, messages: impl IntoIterator<Item = &'a Message>) {
        if self.owner.as_ref() != Some(me) {
            self.files.clear();
            self.owner = Some(me.clone());
        }
        let attachments = messages.into_iter().flat_map(|m| {
            m.attachments
                .iter()
                .chain(m.forwarded.iter().flat_map(|f| f.attachments.iter()))
        });
        for a in attachments {
            let known = self.files.entry(a.file_id.clone()).or_default();
            known.names.entry(a.name.clone()).or_default();
            known.size = known.size.max(a.size);
        }
    }

    /// How file `id` named `name` may be saved for account `me`; `None` when no message of that
    /// account has that file under that name.
    fn rule(&self, me: &UserId, id: &str, name: &str) -> Option<SaveRule> {
        if self.owner.as_ref() != Some(me) {
            return None;
        }
        let known = self.files.get(id)?;
        let saved = known.names.get(name)?.clone();
        let limit = match known.size {
            // Not said: the general limit.
            0 => SAVED_FILE_LIMIT,
            // A little over what the message says, in case its count is a bit off.
            size => size.saturating_add(size / 8 + 64 * 1024).min(SAVED_FILE_LIMIT),
        };
        Some(SaveRule { limit, saved })
    }

    /// File `id` named `name` has been saved at `path`.
    fn saved(&mut self, id: &str, name: &str, path: std::path::PathBuf) {
        if let Some(saved) = self.files.get_mut(id).and_then(|k| k.names.get_mut(name)) {
            *saved = Some(path);
        }
    }

    pub(super) fn clear(&mut self) {
        self.owner = None;
        self.files.clear();
    }
}

/// Records the attachments of messages of account `me` handed to the UI: see [`KnownFiles`].
pub(super) fn note_files<'a>(state: &AppState, me: &UserId, messages: impl IntoIterator<Item = &'a Message>) {
    if let Ok(mut known) = state.known_files.lock() {
        known.note(me, messages);
    }
}

/// A save in progress, counted in [`AppState::saving`] until it ends.
struct Saving<'a>(&'a std::sync::atomic::AtomicUsize);

impl<'a> Saving<'a> {
    fn start(count: &'a std::sync::atomic::AtomicUsize) -> Option<Self> {
        use std::sync::atomic::Ordering;
        count
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < SAVES_AT_ONCE).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(count))
    }
}

impl Drop for Saving<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

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
pub(super) fn prefetch_previews(live: Arc<Live>, messages: &[Message]) {
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

/// Downloads an attachment to "Downloads" and shows it in its folder, without opening it.
/// Returns the file path. Only an attachment of a message the UI got is saved, under its own
/// name (see [`KnownFiles`]), only a few at a time, and only once: a file saved before and still
/// there is shown again instead.
#[tauri::command]
pub async fn save_file(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    args: Args<cmd::SaveFile>,
) -> Result<Out<cmd::SaveFile>, String> {
    let FileArgs { file_id, name } = args;
    let live = live(&state).await?;
    let rule = state
        .known_files
        .lock()
        .ok()
        .and_then(|known| known.rule(&live.me, &file_id, &name))
        .ok_or("not a file from a message")?;
    let path = match rule.saved.filter(|p| p.is_file()) {
        Some(path) => path,
        None => {
            let _saving = Saving::start(&state.saving).ok_or_else(|| {
                text(
                    &app,
                    "Уже скачивается несколько файлов, дождитесь их",
                    "Several files are downloading already, wait for them to finish",
                )
                .to_owned()
            })?;
            let dir = app.path().download_dir().map_err(|e| e.to_string())?;
            let (path, file) = crate::downloads::create_new(&dir, &crate::downloads::safe_name(&name))
                .await
                .map_err(|e| format!("could not save the file: {e}"))?;
            download_to(&live, &file_id, file, &path, rule.limit).await?;
            crate::downloads::mark_from_internet(&path);
            if let Ok(mut known) = state.known_files.lock() {
                known.saved(&file_id, &name, path.clone());
            }
            path
        }
    };
    let shown = path.display().to_string();
    // D-Bus may have to start the file manager first: off the async threads.
    tokio::task::spawn_blocking(move || crate::downloads::reveal(&path))
        .await
        .map_err(|e| e.to_string())??;
    Ok(shown)
}

/// Writes an attachment into `file`, just created at `path`, as it arrives, up to `limit` bytes;
/// a failed download leaves no file behind.
async fn download_to(
    live: &Live,
    file_id: &str,
    mut file: tokio::fs::File,
    path: &std::path::Path,
    limit: u64,
) -> Result<(), String> {
    let downloaded = live.web.download(file_id, &mut file, limit).await;
    drop(file);
    if let Err(e) = downloaded {
        let _ = tokio::fs::remove_file(path).await;
        return Err(e.to_string());
    }
    Ok(())
}

/// "Edit" in your own profile: the name and photo are edited in Yandex ID.
#[tauri::command]
pub fn open_yandex_id() -> Result<Out<cmd::OpenYandexId>, String> {
    open_external("https://id.yandex.ru/personal")
}

/// A link from a message opens in the default browser, if it is safe to (see
/// [`mostik_core::markup::safe_url`]): the address comes from someone else's text.
/// A Telemost meeting link opens in the call window, like a huddle link in Slack.
#[tauri::command]
pub async fn open_url(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    args: Args<cmd::OpenUrl>,
) -> Result<Out<cmd::OpenUrl>, String> {
    let UrlArgs { url } = args;
    if !mostik_core::markup::safe_url(&url) {
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

/// "+" in the composer: the system file picker.
#[tauri::command]
pub async fn pick_files(app: AppHandle) -> Result<Out<cmd::PickFiles>, String> {
    use tauri_plugin_dialog::DialogExt;
    pick(&app, app.dialog().file(), true).await
}

/// Shows the system file picker `dialog`, for several files at once if `many`, and allows sending
/// the files picked (see [`allow_files`]); none if the user cancels.
pub(crate) async fn pick(
    app: &AppHandle,
    dialog: tauri_plugin_dialog::FileDialogBuilder<tauri::Wry>,
    many: bool,
) -> Result<Vec<PendingFile>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if many {
        dialog.pick_files(move |picked| {
            let _ = tx.send(picked.unwrap_or_default());
        });
    } else {
        dialog.pick_file(move |picked| {
            let _ = tx.send(picked.into_iter().collect());
        });
    }
    let picked: Vec<tauri_plugin_dialog::FilePath> = rx.await.map_err(|e| e.to_string())?;
    let paths: Vec<std::path::PathBuf> = picked.into_iter().filter_map(|f| f.into_path().ok()).collect();
    Ok(allow_files(app, &paths))
}

/// Path prefix of the files kept by [`keep_file`]: they have no path on disk.
pub(super) const KEPT: &str = "kept:";

/// The largest file kept in memory by [`keep_file`].
const KEPT_FILE_LIMIT: usize = 200 * 1024 * 1024;

/// The most bytes all kept files take together.
const KEPT_TOTAL_LIMIT: usize = 512 * 1024 * 1024;

/// Whether a file of `size` bytes may be kept next to the `kept` ones.
fn fits(kept: &BTreeMap<String, bytes::Bytes>, size: usize) -> bool {
    let total: usize = kept.values().map(bytes::Bytes::len).sum();
    size <= KEPT_FILE_LIMIT && total.saturating_add(size) <= KEPT_TOTAL_LIMIT
}

/// A file pasted or dropped into the message field: its contents come in the request body, its
/// name (URI-encoded) in the `x-file-name` header. It is kept in memory, so that a pasted
/// screenshot leaves nothing on disk, and is sent like a picked file.
#[tauri::command]
pub async fn keep_file(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<Out<cmd::KeepFile>, String> {
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
    {
        let mut kept = state.kept_files.lock().map_err(|e| e.to_string())?;
        if !fits(&kept, bytes.len()) {
            return Err(text(
                &app,
                "Файл слишком большой, чтобы вставить его: прикрепите его кнопкой «+»",
                "The file is too large to paste: attach it with \"+\"",
            )
            .into());
        }
        // The request owns its body: one copy, which the upload then shares.
        kept.insert(path.clone(), bytes::Bytes::copy_from_slice(bytes));
    }
    if let Ok(mut allowed) = state.allowed_files.lock() {
        allowed.insert(path);
    }
    Ok(file)
}

/// A file taken off the message (× on its card): a pasted one is no longer kept.
#[tauri::command]
pub fn forget_file(state: tauri::State<'_, AppState>, args: Args<cmd::ForgetFile>) -> Out<cmd::ForgetFile> {
    let PathArgs { path } = args;
    if !path.starts_with(KEPT) {
        return;
    }
    if let Ok(mut kept) = state.kept_files.lock() {
        kept.remove(&path);
    }
    if let Ok(mut allowed) = state.allowed_files.lock() {
        allowed.remove(&path);
    }
}

/// Uploads the files and sends them: images as one gallery with a caption, the rest one by one.
/// For non-images the caption goes as a separate text message before them.
#[tauri::command]
pub async fn send_files(
    state: tauri::State<'_, AppState>,
    args: Args<cmd::SendFiles>,
) -> Result<Out<cmd::SendFiles>, String> {
    let SendFilesArgs {
        chat_id: ChatId(chat_id),
        thread,
        paths,
        text,
    } = args;
    let live = live(&state).await?;
    let allowed = state.allowed_files.lock().map(|a| a.clone()).unwrap_or_default();
    if let Some(bad) = paths.iter().find(|p| !allowed.contains(*p)) {
        return Err(format!("the file was not picked by the user: {bad}"));
    }
    let target = target_chat(&chat_id, thread)?;
    let gallery_id = request_id();
    let kept: Vec<Option<bytes::Bytes>> = {
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
        let msg = wire::gallery(&target, &gallery_id, images, &text);
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
    note_files(&state, &live.me, &sent);
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
    kept: Option<bytes::Bytes>,
) -> Result<Uploaded, String> {
    let p = std::path::Path::new(path);
    let bytes = match kept {
        Some(bytes) => bytes,
        None => tokio::fs::read(p).await.map_err(|e| format!("{path}: {e}"))?.into(),
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
    let id = web
        .upload_file(&target, &payload, &name, mime.essence_str(), bytes)
        .await
        .map_err(|e| e.to_string())?;
    let info = wire::file_info(&id, &name, size);
    Ok(match dims {
        Some(d) => Uploaded::Image(wire::gallery_image(d.width as u64, d.height as u64, info)),
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
    let sent = sent_from_reply(&push(live, msg.clone()).await?)?;
    let fake = json!({ "ClientMessage": msg, "ServerMessageInfo": {
        "Timestamp": sent.ts, "SeqNo": sent.seq_no, "From": { "Guid": live.me.0 } } });
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

#[cfg(test)]
mod tests {
    use mostik_core::{Attachment, AttachmentKind, Forwarded, UserId};

    use super::*;

    fn file(id: &str, name: &str, size: u64) -> Attachment {
        Attachment {
            kind: AttachmentKind::File,
            file_id: id.into(),
            name: name.into(),
            size,
            width: 0,
            height: 0,
        }
    }

    fn message(attachments: Vec<Attachment>, forwarded: Vec<Attachment>) -> Message {
        Message {
            chat_id: ChatId("c1".into()),
            ts: 1,
            author: UserId("u1".into()),
            text: String::new(),
            thread_parent: None,
            edited: false,
            important: false,
            reactions: vec![],
            thread: None,
            seq_no: 1,
            attachments,
            forwarded: vec![Forwarded {
                author: UserId("u2".into()),
                author_name: "Ann".into(),
                ts: 0,
                chat_id: None,
                text: String::new(),
                attachments: forwarded,
                quote: None,
                reply: false,
            }],
            call: None,
            system: None,
            reads: None,
        }
    }

    #[test]
    fn only_files_from_messages_are_saved_under_their_names() {
        let (me, other) = (UserId("me".into()), UserId("other".into()));
        let limit = |known: &KnownFiles, id: &str, name: &str| known.rule(&me, id, name).map(|r| r.limit);
        let mut known = KnownFiles::default();
        assert_eq!(limit(&known, "f1", "a.pdf"), None);
        known.note(
            &me,
            [&message(
                vec![file("f1", "a.pdf", 8 * 1024 * 1024), file("f2", "b.bin", 0)],
                vec![file("f3", "c.txt", 10)],
            )],
        );
        // Somewhat over the size the message gives, and the general limit when it gives none.
        assert_eq!(limit(&known, "f1", "a.pdf"), Some(9 * 1024 * 1024 + 64 * 1024));
        assert_eq!(limit(&known, "f2", "b.bin"), Some(SAVED_FILE_LIMIT));
        assert_eq!(limit(&known, "f3", "c.txt"), Some(10 + 1 + 64 * 1024));
        // A known file under another name, and an unknown file.
        assert_eq!(limit(&known, "f1", "a.pdf.exe"), None);
        assert_eq!(limit(&known, "f4", "a.pdf"), None);
        // The same file forwarded under another name is saved under either.
        known.note(&me, [&message(vec![file("f1", "copy.pdf", 8 * 1024 * 1024)], vec![])]);
        assert!(limit(&known, "f1", "copy.pdf").is_some() && limit(&known, "f1", "a.pdf").is_some());
        // A size past the general limit is held to it.
        known.note(&me, [&message(vec![file("f5", "big.iso", u64::MAX)], vec![])]);
        assert_eq!(limit(&known, "f5", "big.iso"), Some(SAVED_FILE_LIMIT));
        // Another account's files are not this one's, and the first file of another account
        // forgets the rest.
        assert_eq!(known.rule(&other, "f1", "a.pdf"), None);
        known.note(&other, [&message(vec![file("f6", "d.txt", 1)], vec![])]);
        assert_eq!(limit(&known, "f1", "a.pdf"), None);
        assert!(known.rule(&other, "f6", "d.txt").is_some());
        known.clear();
        assert_eq!(known.rule(&other, "f6", "d.txt"), None);
    }

    #[test]
    fn a_saved_file_is_remembered_under_its_name() {
        let me = UserId("me".into());
        let mut known = KnownFiles::default();
        known.note(
            &me,
            [&message(vec![file("f1", "a.pdf", 10), file("f1", "b.pdf", 10)], vec![])],
        );
        known.saved("f1", "a.pdf", "/d/a.pdf".into());
        assert_eq!(
            known.rule(&me, "f1", "a.pdf").and_then(|r| r.saved),
            Some("/d/a.pdf".into())
        );
        assert_eq!(known.rule(&me, "f1", "b.pdf").and_then(|r| r.saved), None);
        // A message that comes again does not forget the saved file.
        known.note(&me, [&message(vec![file("f1", "a.pdf", 10)], vec![])]);
        assert_eq!(
            known.rule(&me, "f1", "a.pdf").and_then(|r| r.saved),
            Some("/d/a.pdf".into())
        );
    }

    #[test]
    fn saves_at_once_are_limited() {
        let count = std::sync::atomic::AtomicUsize::new(0);
        let saves: Vec<_> = (0..SAVES_AT_ONCE).map_while(|_| Saving::start(&count)).collect();
        assert_eq!(saves.len(), SAVES_AT_ONCE);
        assert!(Saving::start(&count).is_none());
        drop(saves);
        assert!(Saving::start(&count).is_some());
    }

    #[test]
    fn kept_files_stay_within_the_limits() {
        let mut kept = BTreeMap::new();
        assert!(fits(&kept, KEPT_FILE_LIMIT));
        assert!(!fits(&kept, KEPT_FILE_LIMIT + 1));
        kept.insert("a".to_owned(), bytes::Bytes::from(vec![0; KEPT_TOTAL_LIMIT - 10]));
        assert!(fits(&kept, 10));
        assert!(!fits(&kept, 11));
    }
}
