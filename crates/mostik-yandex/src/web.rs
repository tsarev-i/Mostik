//! HTTP side of the client: the web client page, CSRF and the `/api/` RPC.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::StatusCode;
use reqwest::header::{
    CONTENT_LENGTH, CONTENT_RANGE, COOKIE, HeaderMap, HeaderValue, LOCATION, ORIGIN, REFERER, SET_COOKIE, USER_AGENT,
};
use serde_json::Value;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::endpoints::{API_URL, CSRF_URL, FILES_HOST, WEB_ORIGIN};
use crate::rpc::{FORM_FIELD, RpcRequest};
use crate::session::CookieJar;

/// HTTP transport error.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("network: {0}")]
    Http(#[from] reqwest::Error),
    #[error("session expired: please sign in again")]
    Unauthorized,
    #[error("the Telemost page has no {0} parameter")]
    MissingParam(&'static str),
    #[error("bad server response: {0}")]
    BadResponse(String),
    /// A gateway or server error (5xx): Telemost is overloaded or restarting.
    #[error("{method}: Telemost is not responding (HTTP {status})")]
    Unavailable { method: String, status: u16 },
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Net(#[from] crate::net::NetError),
    /// The file is larger than the caller takes.
    #[error("the file is larger than {} KB", .0 / 1024)]
    TooLarge(u64),
    /// A download got no data for [`STALL_TIMEOUT`].
    #[error("the download stalled")]
    Stalled,
    #[error("could not write the file: {0}")]
    Write(#[from] std::io::Error),
    #[error("could not read the file: {0}")]
    Read(std::io::Error),
    /// Yandex Disk has no room left for a file sent through it (`507`, and `403` as the web
    /// client takes it).
    #[error("there is no room left on Yandex Disk")]
    DiskFull,
    /// The file is larger than Yandex Disk takes (`413`).
    #[error("the file is larger than Yandex Disk takes")]
    DiskTooLarge,
    /// Not a file id that [`valid_file_id`] lets into an address.
    #[error("bad file identifier")]
    BadFileId,
}

impl WebError {
    /// Whether trying again later may help: a timeout, no connection or a 5xx answer.
    pub fn is_transient(&self) -> bool {
        match self {
            WebError::Unavailable { .. } | WebError::Stalled => true,
            WebError::Http(e) => e.is_timeout() || e.is_connect(),
            _ => false,
        }
    }
}

/// How long one API call may take before it counts as failed.
const RPC_TIMEOUT: Duration = Duration::from_secs(20);

/// How long opening a connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one step of loading the page may take.
const PAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a file transfer may wait for the response or for the next piece of the file.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The slowest transfer still waited for, in bytes per second (see [`transfer_timeout`]).
const SLOWEST_TRANSFER: u64 = 32 * 1024;

/// The longest time limit of a transfer.
const MAX_TRANSFER_TIME: Duration = Duration::from_secs(2 * 3600);

/// The time limit of a transfer of `bytes`: a minute plus the time it takes at 32 KB/s, at most
/// two hours. A transfer that stops altogether fails sooner, after [`STALL_TIMEOUT`].
pub fn transfer_timeout(bytes: u64) -> Duration {
    Duration::from_secs(60 + bytes / SLOWEST_TRANSFER).min(MAX_TRANSFER_TIME)
}

/// How long Mostik waits for Yandex Disk to take in a file it has got whole, how often it asks,
/// and the pauses before an upload broken off goes on from where the Disk got to, one per try:
/// growing, so that a minute without a network does not use them all up (#65).
/// `stall` is how long a `PUT` may go without a byte taken before it counts as broken off and
/// goes on like one: shorter than the minute after which the app gives up on a silent upload
/// (`UPLOAD_STALL`), which a connection that died mid-file would otherwise reach first.
#[derive(Debug, Clone, Copy)]
struct DiskTiming {
    processing: Duration,
    poll: Duration,
    resume_pauses: &'static [Duration],
    stall: Duration,
}

const DISK_TIMING: DiskTiming = DiskTiming {
    processing: Duration::from_secs(20 * 60),
    poll: Duration::from_secs(5),
    // Two and a half minutes in all; a try that got further starts them again.
    resume_pauses: &[
        Duration::from_secs(2),
        Duration::from_secs(4),
        Duration::from_secs(8),
        Duration::from_secs(15),
        Duration::from_secs(30),
        Duration::from_secs(30),
        Duration::from_secs(30),
        Duration::from_secs(30),
    ],
    stall: Duration::from_secs(40),
};

/// Pauses before the repeats of a reading call that failed for a transient reason.
const READ_RETRIES: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1500)];

/// Whether an API method only reads, so it is safe to repeat. A write (send, create, leave)
/// is never repeated on its own: the first attempt may have gone through.
fn reads_only(method: &str) -> bool {
    method.starts_with("get_") || method == "search" || method.ends_with("_suggest")
}

/// User parameters from the `globalParams` of the web client page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageParams {
    pub puid: String,
    pub service_id: Option<String>,
    pub logged_in: bool,
}

/// Extracts the value of `"key": "…"` or `"key": 123` from the page HTML.
fn find_param(html: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = html.find(&needle)? + needle.len();
    let rest = html[start..].trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        quoted.find('"').map(|end| quoted[..end].to_owned())
    } else {
        let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))?;
        Some(rest[..end].to_owned())
    }
}

/// Reads the user parameters from the web client page.
fn parse_page_params(html: &str) -> Result<PageParams, WebError> {
    let logged_in = find_param(html, "loggedIn").as_deref() == Some("true");
    let puid = find_param(html, "puid")
        .filter(|p| !p.is_empty() && p != "null")
        .ok_or(WebError::MissingParam("puid"))?;
    Ok(PageParams {
        puid,
        service_id: find_param(html, "serviceId").filter(|s| !s.is_empty() && s != "null"),
        logged_in,
    })
}

/// Extracts the CSRF token from the `/csrf-token/` response; accepts several layouts.
fn parse_csrf(body: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        for key in ["token", "csrf_token", "csrf", "csrfToken"] {
            if let Some(t) = v.get(key).and_then(Value::as_str) {
                return Some(t.to_owned());
            }
            if let Some(t) = v.get("data").and_then(|d| d.get(key)).and_then(Value::as_str) {
                return Some(t.to_owned());
            }
        }
        return v.as_str().map(str::to_owned);
    }
    let t = body.trim();
    (!t.is_empty() && !t.contains(char::is_whitespace) && t.len() < 512).then(|| t.to_owned())
}

/// HTTP client with the session cookies, CSRF token and page parameters.
pub struct WebClient {
    http: reqwest::Client,
    /// For the page only: it follows redirects itself to take the cookies they set.
    page_http: reqwest::Client,
    jar: CookieJar,
    user_agent: String,
    /// Shared by all `clone_light` copies, so a refreshed token reaches every one of them.
    csrf: Arc<Mutex<Option<String>>>,
    pub page: Option<PageParams>,
    /// Status, host and path of every response of the last `load_page`, for the probe.
    pub page_hops: Vec<String>,
}

/// Redirects `load_page` follows before giving up: Telemost sends to `cookier.360.yandex.ru` and
/// back, Yandex ID may add a hop or two to refresh the session.
const PAGE_REDIRECTS: usize = 8;

impl WebClient {
    /// Both clients share the TLS setup of the socket ([`crate::net`]). Every request has a time
    /// limit: an API call or the token [`RPC_TIMEOUT`], a page step [`PAGE_TIMEOUT`], a file one
    /// that grows with its size ([`transfer_timeout`]).
    pub fn new(jar: CookieJar, user_agent: String) -> Result<Self, WebError> {
        let tls = crate::net::http_tls()?;
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls.clone())
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .build()?;
        let page_http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(PAGE_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            page_http,
            jar,
            user_agent,
            csrf: Arc::default(),
            page: None,
            page_hops: Vec::new(),
        })
    }

    /// Copy of the client for parallel requests: shared connection pool, CSRF and page parameters.
    pub fn clone_light(&self) -> Self {
        Self {
            http: self.http.clone(),
            page_http: self.page_http.clone(),
            jar: self.jar.clone(),
            user_agent: self.user_agent.clone(),
            csrf: self.csrf.clone(),
            page: self.page.clone(),
            page_hops: Vec::new(),
        }
    }

    pub fn jar(&self) -> &CookieJar {
        &self.jar
    }

    fn headers(&self, host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&self.jar.header_for(host)) {
            h.insert(COOKIE, v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.user_agent) {
            h.insert(USER_AGENT, v);
        }
        h.insert(ORIGIN, HeaderValue::from_static(WEB_ORIGIN));
        h.insert(REFERER, HeaderValue::from_static("https://telemost.360.yandex.ru/"));
        h
    }

    /// Loads the web client page and takes `puid` and `serviceId` from it. Redirects are
    /// followed here, with the cookies they set going into the jar: Telemost sends a session
    /// without a fresh `yandex360session` to `cookier.360.yandex.ru` and back, and replaying the
    /// old cookies would loop. The caller saves the jar if it changed.
    pub async fn load_page(&mut self) -> Result<PageParams, WebError> {
        let mut url =
            reqwest::Url::parse(&format!("{WEB_ORIGIN}/")).map_err(|e| WebError::BadResponse(e.to_string()))?;
        self.page_hops.clear();
        for _ in 0..=PAGE_REDIRECTS {
            let host = url.host_str().unwrap_or_default().to_owned();
            let resp = self
                .page_http
                .get(url.clone())
                .headers(self.headers(&host))
                .send()
                .await?;
            let status = resp.status();
            self.page_hops.push(format!("{} {host}{}", status.as_u16(), url.path()));
            for header in resp.headers().get_all(SET_COOKIE) {
                if let Ok(header) = header.to_str() {
                    self.jar.set_cookie(&host, header);
                }
            }
            if status.is_redirection() {
                url = resp
                    .headers()
                    .get(LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .and_then(|l| url.join(l).ok())
                    .filter(|next| next.scheme() == "https")
                    .ok_or_else(|| WebError::BadResponse(format!("page: HTTP {status} without a usable Location")))?;
                continue;
            }
            let html = resp.text().await?;
            let params = parse_page_params(&html)?;
            if !params.logged_in {
                return Err(WebError::Unauthorized);
            }
            self.page = Some(params.clone());
            return Ok(params);
        }
        Err(WebError::BadResponse("page: too many redirects".into()))
    }

    /// CSRF token for write requests; fetched once and cached until the server rejects it.
    async fn csrf_token(&self) -> Result<String, WebError> {
        if let Some(t) = self.csrf.lock().ok().and_then(|c| c.clone()) {
            return Ok(t);
        }
        let resp = self
            .http
            .post(CSRF_URL)
            .headers(self.headers("messenger.360.yandex.ru"))
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(WebError::Unauthorized);
        }
        let token = parse_csrf(&body).ok_or_else(|| WebError::BadResponse(format!("csrf: HTTP {status}")))?;
        if let Ok(mut c) = self.csrf.lock() {
            *c = Some(token.clone());
        }
        Ok(token)
    }

    /// Forgets a token the server rejected, unless another request has already replaced it.
    fn invalidate_csrf(&self, rejected: &str) {
        if let Ok(mut c) = self.csrf.lock()
            && c.as_deref() == Some(rejected)
        {
            *c = None;
        }
    }

    /// A file or preview of the Messenger storage, whole, with its `Content-Type`. A file larger
    /// than `limit` bytes is refused.
    pub async fn fetch_file(
        &self,
        file_id: &str,
        size: Option<&str>,
        limit: u64,
    ) -> Result<(Vec<u8>, String), WebError> {
        self.fetch(file_url(file_id, size)?, limit, STALL_TIMEOUT).await
    }

    /// Writes a file of the Messenger storage into `out` as it arrives. A file larger than `limit`
    /// bytes is refused, or stopped once that much has come. Returns the number of bytes written.
    pub async fn download(
        &self,
        file_id: &str,
        out: &mut (impl AsyncWrite + Unpin),
        limit: u64,
    ) -> Result<u64, WebError> {
        let mut download = self.open(file_url(file_id, None)?, limit, STALL_TIMEOUT).await?;
        while let Some(chunk) = download.next().await? {
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        Ok(download.got)
    }

    /// [`WebClient::fetch_file`] from `url`, with the given wait for data.
    async fn fetch(&self, url: reqwest::Url, limit: u64, stall: Duration) -> Result<(Vec<u8>, String), WebError> {
        let mut download = self.open(url, limit, stall).await?;
        let mut bytes = Vec::with_capacity(download.expected as usize);
        while let Some(chunk) = download.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        Ok((bytes, download.kind))
    }

    /// Requests a file; refuses it at once when its `Content-Length` is over `limit`.
    async fn open(&self, url: reqwest::Url, limit: u64, stall: Duration) -> Result<Download, WebError> {
        let request = self
            .http
            .get(url)
            .headers(self.headers(FILES_HOST))
            .timeout(transfer_timeout(limit))
            .send();
        let resp = tokio::time::timeout(stall, request)
            .await
            .map_err(|_| WebError::Stalled)??;
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(WebError::Unauthorized);
        }
        if !status.is_success() {
            return Err(WebError::BadResponse(format!("file: HTTP {status}")));
        }
        let expected = resp.content_length().unwrap_or(0);
        if expected > limit {
            return Err(WebError::TooLarge(limit));
        }
        let kind = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        Ok(Download {
            resp,
            kind,
            expected,
            limit,
            got: 0,
            stall,
        })
    }

    /// File upload: `POST media_upload/{chatId}/{messageId}/{filename}`, the body is the file bytes.
    /// `message_id` is the UUID of the future message (also its `PayloadId`). Returns the `file_id`
    /// of the uploaded file (`Id2` in [`crate::wire::file_info`]).
    pub async fn upload_file(
        &mut self,
        chat_id: &str,
        message_id: &str,
        filename: &str,
        content_type: &str,
        bytes: bytes::Bytes,
    ) -> Result<String, WebError> {
        self.upload_file_with_progress(chat_id, message_id, filename, content_type, bytes, Arc::new(|_| {}))
            .await
    }

    /// [`WebClient::upload_file`] that tells `progress` how many bytes of the file have gone, as
    /// they go; a repeat after a stale CSRF token starts from 0 again.
    pub async fn upload_file_with_progress(
        &mut self,
        chat_id: &str,
        message_id: &str,
        filename: &str,
        content_type: &str,
        bytes: bytes::Bytes,
        progress: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Result<String, WebError> {
        // Like `encodeURI` in the web client: the chat ID goes in with its slashes as is,
        // only the file name is encoded (and a `/` in it is escaped so it cannot break the path).
        let url = format!(
            "https://{}/media_upload/{chat_id}/{message_id}/{}",
            FILES_HOST,
            encode_uri_component(filename)
        );
        let mut retried = false;
        let (status, body) = loop {
            let csrf = self.csrf_token().await?;
            let mut headers = self.headers(FILES_HOST);
            if let Ok(v) = HeaderValue::from_str(&csrf) {
                headers.insert("X-CSRF-TOKEN", v);
            }
            if let Ok(v) = HeaderValue::from_str(content_type) {
                headers.insert(reqwest::header::CONTENT_TYPE, v);
            }
            let body = UploadBody {
                bytes: bytes.clone(),
                sent: 0,
                progress: progress.clone(),
            };
            let resp = self
                .http
                .post(&url)
                .headers(headers)
                .body(reqwest::Body::wrap(body))
                .timeout(transfer_timeout(bytes.len() as u64))
                .send()
                .await?;
            let status = resp.status();
            let body = resp.text().await?;
            if crate::diag::traffic_logging() {
                let logged = serde_json::from_str::<Value>(&body).unwrap_or_else(|_| Value::String(body.clone()));
                crate::diag::traffic_log(serde_json::json!({
                    "dir": "in", "via": "http", "upload": url, "status": status.as_u16(), "body": logged
                }));
            }
            // A 403 usually means a stale CSRF token: retry once with a fresh one.
            if status == StatusCode::FORBIDDEN && !retried {
                self.invalidate_csrf(&csrf);
                retried = true;
                continue;
            }
            break (status, body);
        };
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(WebError::Unauthorized);
        }
        let v: Value =
            serde_json::from_str(&body).map_err(|_| WebError::BadResponse(format!("upload: HTTP {status}")))?;
        if !status.is_success() {
            return Err(WebError::BadResponse(format!(
                "upload: HTTP {status} {}",
                v.pointer("/data/code").and_then(Value::as_str).unwrap_or("")
            )));
        }
        v.get("data")
            .unwrap_or(&v)
            .get("file_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| WebError::BadResponse("Telemost did not return file_id".into()))
    }

    /// Where on the user's Yandex Disk a file goes that is too large for the Messenger storage
    /// (`upload_to_disk`, as the web client does it): an address that takes the file without
    /// cookies. `name` and `size` are the file's, `chat_id` is the chat it is sent to; a `video`
    /// gets a preview from its first frame.
    pub async fn disk_upload_url(
        &mut self,
        chat_id: &str,
        name: &str,
        size: u64,
        video: bool,
    ) -> Result<String, WebError> {
        let mut file = serde_json::json!({
            "name": name, "upload_id": crate::wire::request_id(), "size": size, "chat_id": chat_id
        });
        if video {
            file["preview_timecode"] = 0.into();
        }
        let v = self
            .rpc("upload_to_disk", serde_json::json!({ "files": [file] }))
            .await?;
        if let Some(refused) = disk_refusal(&v) {
            return Err(refused);
        }
        v.pointer("/data/files/0/upload_url")
            .and_then(Value::as_str)
            .filter(|url| url.starts_with("https://"))
            .map(str::to_owned)
            .ok_or_else(|| WebError::BadResponse(format!("upload_to_disk: no upload address {}", error_code(&v))))
    }

    /// Puts a file of `size` bytes to `url` from [`WebClient::disk_upload_url`], telling
    /// `progress` how many bytes the Disk has, and waits while Yandex Disk takes it in. An upload
    /// broken off goes on from where the Disk got to, with growing pauses. With `resume` the
    /// address was used before (a retry of the sending): the Disk is asked first how much it has.
    /// Returns the file's place on the Disk (`Location`) for [`WebClient::add_disk_file`].
    pub async fn disk_put(
        &self,
        url: &str,
        file: &DiskFile,
        size: u64,
        resume: bool,
        progress: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Result<String, WebError> {
        self.disk_put_timed(url, file, size, resume, progress, DISK_TIMING)
            .await
    }

    async fn disk_put_timed(
        &self,
        url: &str,
        file: &DiskFile,
        size: u64,
        resume: bool,
        progress: Arc<dyn Fn(u64) + Send + Sync>,
        timing: DiskTiming,
    ) -> Result<String, WebError> {
        if size == 0 {
            return Err(WebError::BadResponse("disk upload: the file is empty".into()));
        }
        // `None`: how much the Disk has is to be asked before going on.
        let mut from = (!resume).then_some(0);
        let mut failures = 0;
        // How far the Disk had got at the last failure: a try that got further is not counted.
        let mut reached = 0;
        // How far the Disk is known to have got.
        let mut known = 0;
        // Where the body of the `PUT` going on is, and when it last moved (ms since `start`).
        let start = tokio::time::Instant::now();
        let (body_at, moved) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let watched: Arc<dyn Fn(u64) + Send + Sync> = {
            let (body_at, moved, progress) = (body_at.clone(), moved.clone(), progress.clone());
            Arc::new(move |sent| {
                body_at.store(sent, Ordering::SeqCst);
                moved.store(start.elapsed().as_millis() as u64, Ordering::SeqCst);
                progress(sent);
            })
        };
        loop {
            let step = match from {
                None => self.disk_offset(url).await.map(|got| {
                    let got = got.min(size);
                    progress(got);
                    known = got;
                    if got > reached {
                        (reached, failures) = (got, 0);
                    }
                    from = Some(got);
                    None
                }),
                Some(at) if at < size => {
                    known = at;
                    body_at.store(at, Ordering::SeqCst);
                    moved.store(start.elapsed().as_millis() as u64, Ordering::SeqCst);
                    // A body that stops before its end is a break, not an upload to wait for.
                    let stalled = async {
                        loop {
                            tokio::time::sleep(timing.stall / 10).await;
                            let still =
                                (start.elapsed().as_millis() as u64).saturating_sub(moved.load(Ordering::SeqCst));
                            if body_at.load(Ordering::SeqCst) < size && still >= timing.stall.as_millis() as u64 {
                                break;
                            }
                        }
                    };
                    tokio::select! {
                        answer = self.disk_put_part(url, file, size, at, watched.clone()) => answer.map(Some),
                        () = stalled => Err(WebError::Stalled),
                    }
                }
                Some(_) => Ok(Some(DiskAnswer::Processing)),
            };
            match step {
                Ok(None) => {}
                Ok(Some(DiskAnswer::Done(location))) => return Ok(location),
                Ok(Some(DiskAnswer::Processing)) => return self.disk_wait(url, size, &progress, timing).await,
                Err(e) if disk_retry(&e) && failures < timing.resume_pauses.len() => {
                    // A sign of life for the caller, whose watch for a silent upload would end it
                    // while the tries wait: a pause and one try take less than its minute.
                    progress(known);
                    tokio::time::sleep(timing.resume_pauses[failures]).await;
                    failures += 1;
                    from = None;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// One `PUT` of the file from byte `from` to its end, with its `Content-Range`.
    async fn disk_put_part(
        &self,
        url: &str,
        file: &DiskFile,
        size: u64,
        from: u64,
        progress: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Result<DiskAnswer, WebError> {
        let body = match file {
            DiskFile::Memory(bytes) => {
                if bytes.len() as u64 != size {
                    return Err(WebError::BadResponse("disk upload: the file changed".into()));
                }
                let progress = progress.clone();
                reqwest::Body::wrap(UploadBody {
                    bytes: bytes.slice(from as usize..),
                    sent: 0,
                    progress: Arc::new(move |sent| progress(from + sent)),
                })
            }
            DiskFile::Path(path) => {
                use tokio::io::AsyncSeekExt;
                let mut f = tokio::fs::File::open(path).await.map_err(WebError::Read)?;
                f.seek(std::io::SeekFrom::Start(from)).await.map_err(WebError::Read)?;
                reqwest::Body::wrap(FileBody {
                    file: f,
                    left: size - from,
                    sent: from,
                    buf: vec![0; UPLOAD_CHUNK],
                    progress,
                })
            }
        };
        let mut headers = self.disk_headers();
        if let Ok(range) = HeaderValue::from_str(&format!("bytes {from}-{}/{size}", size - 1)) {
            headers.insert(CONTENT_RANGE, range);
        }
        let resp = self
            .http
            .put(url)
            .headers(headers)
            .body(body)
            .timeout(transfer_timeout(size - from))
            .send()
            .await?;
        disk_logged(resp.status(), "put");
        disk_answer(resp.status(), resp.headers())
    }

    /// How many bytes of the file Yandex Disk has got: `Content-Length` of a `HEAD`.
    async fn disk_offset(&self, url: &str) -> Result<u64, WebError> {
        let resp = self.http.head(url).headers(self.disk_headers()).send().await?;
        let status = resp.status();
        disk_logged(status, "head");
        if !status.is_success() {
            return Err(disk_error(status));
        }
        Ok(resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    /// Asks with an empty `PUT` until Yandex Disk has taken in the whole file (`201`), every
    /// `timing.poll`, for `timing.processing` at most. Each ask tells `progress` that the upload
    /// is alive.
    async fn disk_wait(
        &self,
        url: &str,
        size: u64,
        progress: &Arc<dyn Fn(u64) + Send + Sync>,
        timing: DiskTiming,
    ) -> Result<String, WebError> {
        let deadline = tokio::time::Instant::now() + timing.processing;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return Err(WebError::BadResponse(
                    "Yandex Disk did not take in the file in time".into(),
                ));
            }
            let asked = self
                .http
                .put(url)
                .headers(self.disk_headers())
                .body(Vec::new())
                .timeout(left.min(RPC_TIMEOUT))
                .send()
                .await
                .map_err(WebError::from)
                .and_then(|resp| {
                    disk_logged(resp.status(), "wait");
                    disk_answer(resp.status(), resp.headers())
                });
            match asked {
                Ok(DiskAnswer::Done(location)) => return Ok(location),
                Ok(DiskAnswer::Processing) => {}
                Err(e) if disk_retry(&e) => {}
                Err(e) => return Err(e),
            }
            progress(size);
            tokio::time::sleep(timing.poll.min(left)).await;
        }
    }

    /// Headers for Yandex Disk: no cookies, the upload address itself lets the file in.
    fn disk_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&self.user_agent) {
            h.insert(USER_AGENT, v);
        }
        h.insert(ORIGIN, HeaderValue::from_static(WEB_ORIGIN));
        h.insert(REFERER, HeaderValue::from_static("https://telemost.360.yandex.ru/"));
        h
    }

    /// Adds a file put on Yandex Disk (`Location` from [`WebClient::disk_put`]) to chat `chat_id`
    /// (`add_files`). Returns its `file_id` for the message, as [`WebClient::upload_file`] does.
    pub async fn add_disk_file(&mut self, chat_id: &str, location: &str) -> Result<String, WebError> {
        let v = self
            .rpc(
                "add_files",
                serde_json::json!({ "chat_id": chat_id, "files": [{ "location": location }] }),
            )
            .await?;
        v.pointer("/data/files/0/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| WebError::BadResponse(format!("add_files: no file {}", error_code(&v))))
    }

    /// All the synced settings (`get_buckets` from version 0); [`crate::convert::bucket`] finds
    /// one of them.
    pub async fn buckets(&mut self) -> Result<Value, WebError> {
        self.rpc("get_buckets", serde_json::json!({ "version": 0 })).await
    }

    /// `POST /api/` call in the web client format. Returns the whole response JSON. A reading
    /// call that times out or gets a 5xx answer is repeated twice, after a pause.
    pub async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, WebError> {
        let request = RpcRequest::new(method, params).form_value()?;
        if crate::diag::traffic_logging() {
            let body = serde_json::from_str::<Value>(&request).unwrap_or_else(|_| Value::String(request.clone()));
            crate::diag::traffic_log(
                serde_json::json!({ "dir": "out", "via": "http", "method": method, "body": body }),
            );
        }
        let retries = if reads_only(method) { READ_RETRIES.len() } else { 0 };
        let mut attempt = 0;
        loop {
            match self.rpc_once(method, &request, attempt).await {
                Err(e) if e.is_transient() && attempt < retries => {
                    tokio::time::sleep(READ_RETRIES[attempt]).await;
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    /// One attempt of [`WebClient::rpc`]; `attempt` counts the earlier ones.
    async fn rpc_once(&mut self, method: &str, request: &str, attempt: usize) -> Result<Value, WebError> {
        let mut retried = false;
        loop {
            let csrf = self.csrf_token().await?;
            let form = reqwest::multipart::Form::new().text(FORM_FIELD, request.to_owned());
            let mut headers = self.headers("messenger.360.yandex.ru");
            if let Ok(v) = HeaderValue::from_str(&csrf) {
                headers.insert("X-CSRF-TOKEN", v);
            }
            headers.insert("X-Request-Attempt", HeaderValue::from(attempt + usize::from(retried)));
            if let Some(id) = self.page.as_ref().and_then(|p| p.service_id.as_deref())
                && let Ok(v) = HeaderValue::from_str(id)
            {
                headers.insert("X-Origin-Service-ID", v);
            }
            let resp = self
                .http
                .post(API_URL)
                .headers(headers)
                .multipart(form)
                .timeout(RPC_TIMEOUT)
                .send()
                .await?;
            let status = resp.status();
            let body = resp.text().await?;
            if crate::diag::traffic_logging() {
                let logged = serde_json::from_str::<Value>(&body).unwrap_or_else(|_| Value::String(body.clone()));
                crate::diag::traffic_log(serde_json::json!({
                    "dir": "in", "via": "http", "method": method, "status": status.as_u16(), "body": logged
                }));
            }
            if status == StatusCode::UNAUTHORIZED {
                return Err(WebError::Unauthorized);
            }
            let parsed = serde_json::from_str::<Value>(&body);
            // The server rejects a stale CSRF token with a 403 or a `csrf…` error code: retry once with a fresh one.
            if !retried && csrf_rejected(status, parsed.as_ref().ok()) {
                self.invalidate_csrf(&csrf);
                retried = true;
                continue;
            }
            if status.is_server_error() && parsed.is_err() {
                return Err(WebError::Unavailable {
                    method: method.to_owned(),
                    status: status.as_u16(),
                });
            }
            return parsed.map_err(|_| WebError::BadResponse(format!("{method}: HTTP {status}, not JSON")));
        }
    }
}

/// What a URI component keeps as it is: letters, digits and `-_.~()!'`, as `encodeURIComponent`
/// does but for `*`.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'(')
    .remove(b')')
    .remove(b'!')
    .remove(b'\'');

/// `s` percent-encoded as one URI component: nothing in it can end or split the URL around it.
pub fn encode_uri_component(s: &str) -> String {
    utf8_percent_encode(s, URI_COMPONENT).to_string()
}

/// `s` with its `%XX` escapes decoded. A broken escape stays as it is, and bytes that are not
/// UTF-8 become U+FFFD.
pub fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// Whether `id` may be the `Id2` of a file of the Messenger storage in an address of the files
/// host: path segments (`disk/…`), none of them empty, `.` or `..`, without whitespace, control
/// characters, `\`, `?`, `#` or `%`. Every address of a file takes its id through here, so an id
/// from a message (or from a script in the window) cannot reach another path of the host.
pub fn valid_file_id(id: &str) -> bool {
    let bad = |c: char| c.is_whitespace() || c.is_control() || matches!(c, '\\' | '?' | '#' | '%');
    !id.is_empty()
        && id.len() <= 512
        && id
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != ".." && !seg.contains(bad))
}

/// Address of a file or preview of the Messenger storage: `file_shortterm/<Id2>[?size=…]`, the id
/// checked by [`valid_file_id`] and put in segment by segment, the size as a query parameter.
fn file_url(file_id: &str, size: Option<&str>) -> Result<reqwest::Url, WebError> {
    if !valid_file_id(file_id) {
        return Err(WebError::BadFileId);
    }
    let mut url = reqwest::Url::parse(&format!("https://{FILES_HOST}/file_shortterm/"))
        .map_err(|e| WebError::BadResponse(e.to_string()))?;
    url.path_segments_mut()
        .map_err(|()| WebError::BadFileId)?
        .pop_if_empty()
        .extend(file_id.split('/'));
    if let Some(size) = size {
        url.query_pairs_mut().append_pair("size", size);
    }
    Ok(url)
}

/// A file download in progress, read piece by piece so that a file over its limit is dropped
/// as soon as it gets there and a stalled one fails instead of waiting forever.
struct Download {
    resp: reqwest::Response,
    /// `Content-Type` of the file.
    kind: String,
    /// `Content-Length`, 0 when unknown.
    expected: u64,
    limit: u64,
    got: u64,
    stall: Duration,
}

impl Download {
    /// The next piece of the file, `None` after the last one.
    async fn next(&mut self) -> Result<Option<bytes::Bytes>, WebError> {
        let chunk = tokio::time::timeout(self.stall, self.resp.chunk())
            .await
            .map_err(|_| WebError::Stalled)??;
        if let Some(chunk) = &chunk {
            self.got += chunk.len() as u64;
            if self.got > self.limit {
                return Err(WebError::TooLarge(self.limit));
            }
        }
        Ok(chunk)
    }
}

/// How many bytes of an upload go at a time: its progress moves by this much.
const UPLOAD_CHUNK: usize = 64 * 1024;

/// The body of an upload: the file in pieces, each one telling `progress` how many bytes have
/// gone. The connection takes a piece only when it can send it, so the count follows the network.
struct UploadBody {
    bytes: bytes::Bytes,
    sent: usize,
    progress: Arc<dyn Fn(u64) + Send + Sync>,
}

impl http_body::Body for UploadBody {
    type Data = bytes::Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<bytes::Bytes>, Self::Error>>> {
        let this = self.get_mut();
        if this.sent >= this.bytes.len() {
            return std::task::Poll::Ready(None);
        }
        let end = (this.sent + UPLOAD_CHUNK).min(this.bytes.len());
        let piece = this.bytes.slice(this.sent..end);
        this.sent = end;
        (this.progress)(end as u64);
        std::task::Poll::Ready(Some(Ok(http_body::Frame::data(piece))))
    }

    fn is_end_stream(&self) -> bool {
        self.sent >= self.bytes.len()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact((self.bytes.len() - self.sent) as u64)
    }
}

/// What a file sent through Yandex Disk is read from: memory (a pasted file) or its place on the
/// disk, read as it goes.
pub enum DiskFile {
    Memory(bytes::Bytes),
    Path(std::path::PathBuf),
}

/// How Yandex Disk answered a `PUT`.
enum DiskAnswer {
    /// It has the whole file: its place for `add_files`.
    Done(String),
    /// It has the whole file and is taking it in (`202`): ask again later.
    Processing,
}

/// A `PUT` answer: `202` is [`DiskAnswer::Processing`], another 2xx has the file's place in
/// `Location`.
fn disk_answer(status: StatusCode, headers: &HeaderMap) -> Result<DiskAnswer, WebError> {
    if status == StatusCode::ACCEPTED {
        return Ok(DiskAnswer::Processing);
    }
    if !status.is_success() {
        return Err(disk_error(status));
    }
    headers
        .get(LOCATION)
        .and_then(|l| l.to_str().ok())
        .filter(|l| !l.is_empty())
        .map(|l| DiskAnswer::Done(l.to_owned()))
        .ok_or_else(|| WebError::BadResponse(format!("disk upload: HTTP {status} without Location")))
}

/// The error of a failed answer of Yandex Disk.
fn disk_error(status: StatusCode) -> WebError {
    disk_status(u64::from(status.as_u16())).unwrap_or_else(|| {
        if status.is_server_error() {
            WebError::Unavailable {
                method: "disk upload".into(),
                status: status.as_u16(),
            }
        } else {
            WebError::BadResponse(format!("disk upload: HTTP {status}"))
        }
    })
}

/// The refusals of Yandex Disk the user can do something about, by their HTTP status.
fn disk_status(status: u64) -> Option<WebError> {
    match status {
        403 | 507 => Some(WebError::DiskFull),
        413 => Some(WebError::DiskTooLarge),
        _ => None,
    }
}

/// A refusal of Yandex Disk passed on by an RPC answer: `data.source` is `disk`, `data.code` its
/// HTTP status.
fn disk_refusal(v: &Value) -> Option<WebError> {
    if v.pointer("/data/source").and_then(Value::as_str) != Some("disk") {
        return None;
    }
    let code = v.pointer("/data/code")?;
    let code = code.as_u64().or_else(|| code.as_str()?.parse().ok())?;
    disk_status(code)
}

/// Whether an upload to Yandex Disk may go on after `e`: the connection broke or stalled, or the
/// Disk answered 5xx.
fn disk_retry(e: &WebError) -> bool {
    matches!(e, WebError::Http(_) | WebError::Unavailable { .. } | WebError::Stalled)
}

/// `data.code` of an RPC answer, for an error message.
fn error_code(v: &Value) -> String {
    v.pointer("/data/code").map(|c| format!("({c})")).unwrap_or_default()
}

/// Writes the status of an answer of Yandex Disk to the traffic log, without the address: it lets
/// anyone in.
fn disk_logged(status: StatusCode, step: &str) {
    if crate::diag::traffic_logging() {
        crate::diag::traffic_log(serde_json::json!({
            "dir": "in", "via": "http", "disk": step, "status": status.as_u16()
        }));
    }
}

/// The body of an upload read from a file as it goes, from where `sent` says on, `left` bytes;
/// each piece tells `progress` how many bytes of the file have gone. A file shorter than that
/// fails the upload.
struct FileBody {
    file: tokio::fs::File,
    left: u64,
    sent: u64,
    buf: Vec<u8>,
    progress: Arc<dyn Fn(u64) + Send + Sync>,
}

impl http_body::Body for FileBody {
    type Data = bytes::Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<bytes::Bytes>, Self::Error>>> {
        use std::task::Poll;
        use tokio::io::AsyncRead;
        let this = self.get_mut();
        if this.left == 0 {
            return Poll::Ready(None);
        }
        let want = this.buf.len().min(usize::try_from(this.left).unwrap_or(usize::MAX));
        let mut read = tokio::io::ReadBuf::new(&mut this.buf[..want]);
        match std::pin::Pin::new(&mut this.file).poll_read(cx, &mut read) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Ready(Ok(())) => {
                let piece = bytes::Bytes::copy_from_slice(read.filled());
                if piece.is_empty() {
                    return Poll::Ready(Some(Err(std::io::ErrorKind::UnexpectedEof.into())));
                }
                this.left -= piece.len() as u64;
                this.sent += piece.len() as u64;
                (this.progress)(this.sent);
                Poll::Ready(Some(Ok(http_body::Frame::data(piece))))
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.left == 0
    }

    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(self.left)
    }
}

/// Whether a response says the CSRF token was not accepted.
fn csrf_rejected(status: StatusCode, body: Option<&Value>) -> bool {
    status == StatusCode::FORBIDDEN
        || body
            .and_then(|v| v.pointer("/data/code"))
            .and_then(Value::as_str)
            .is_some_and(|code| code.to_ascii_lowercase().contains("csrf"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_reading_calls_are_repeated() {
        for read in [
            "get_users_data",
            "get_members_and_contacts_suggest",
            "search",
            "get_buckets",
        ] {
            assert!(super::reads_only(read), "{read}");
        }
        for write in ["create_chat", "set_bucket", "leave", "create_meeting", "activate_chat"] {
            assert!(!super::reads_only(write), "{write}");
        }
    }

    use super::*;

    #[test]
    fn an_upload_goes_in_pieces_with_its_progress() {
        use http_body::Body;
        let len = UPLOAD_CHUNK * 2 + 10;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let mut body = UploadBody {
            bytes: bytes::Bytes::from(vec![7u8; len]),
            sent: 0,
            progress: Arc::new(move |n| log.lock().unwrap().push(n)),
        };
        assert_eq!(body.size_hint().exact(), Some(len as u64));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut got = 0;
        while let std::task::Poll::Ready(Some(Ok(frame))) = std::pin::Pin::new(&mut body).poll_frame(&mut cx) {
            got += frame.into_data().map_or(0, |d| d.len());
        }
        assert_eq!(got, len);
        assert!(body.is_end_stream());
        let chunk = UPLOAD_CHUNK as u64;
        assert_eq!(*seen.lock().unwrap(), [chunk, 2 * chunk, len as u64]);
    }

    #[test]
    fn detects_rejected_csrf() {
        let ok = serde_json::json!({"status": "ok", "data": {}});
        let bad = serde_json::json!({"status": "error", "data": {"code": "csrf_token_invalid"}});
        assert!(!csrf_rejected(StatusCode::OK, Some(&ok)));
        assert!(csrf_rejected(StatusCode::OK, Some(&bad)));
        assert!(csrf_rejected(StatusCode::FORBIDDEN, None));
    }

    #[test]
    fn parses_global_params() {
        let html = r#"<script>window.globalParams={"serviceId":"abc-1","puid":"1130000000001","loggedIn":true,"x":null}</script>"#;
        assert_eq!(
            parse_page_params(html).unwrap(),
            PageParams {
                puid: "1130000000001".into(),
                service_id: Some("abc-1".into()),
                logged_in: true
            }
        );
        let numeric = r#"{"puid":1130000000001,"loggedIn":false}"#;
        let p = parse_page_params(numeric).unwrap();
        assert_eq!(p.puid, "1130000000001");
        assert!(!p.logged_in);
        assert!(matches!(
            parse_page_params("<html></html>"),
            Err(WebError::MissingParam("puid"))
        ));
    }

    /// Answers one request on a local port with `parts`, a moment apart, then keeps the
    /// connection open for `hang`. Returns the address to ask.
    async fn serve_once(parts: Vec<Vec<u8>>, hang: Duration) -> String {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = s.read(&mut request).await;
            for part in parts {
                let _ = s.write_all(&part).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(hang).await;
        });
        url
    }

    fn client() -> WebClient {
        WebClient::new(CookieJar::default(), "Mostik/test".into()).unwrap()
    }

    /// A chunked answer of `pieces` pieces of 40 bytes.
    fn chunked(pieces: usize) -> Vec<Vec<u8>> {
        let mut parts =
            vec![b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec()];
        for _ in 0..pieces {
            parts.push([b"28\r\n".as_slice(), &[b'x'; 40], b"\r\n"].concat());
        }
        parts.push(b"0\r\n\r\n".to_vec());
        parts
    }

    #[test]
    fn file_ids_stay_in_their_path() {
        let url = |id, size| file_url(id, size).map(|u| u.to_string());
        assert_eq!(
            url("disk/x1", Some("orig")).unwrap(),
            "https://files.messenger.yandex.ru/file_shortterm/disk/x1?size=orig"
        );
        for bad in [
            "",
            "../../x",
            "disk/../x",
            "disk/./x",
            "disk//x",
            "/x",
            "x/",
            "a?b",
            "a#b",
            "a b",
            "a%2e",
            "a\\b",
            "a\nb",
        ] {
            assert!(!valid_file_id(bad), "{bad:?}");
            assert!(matches!(file_url(bad, None), Err(WebError::BadFileId)), "{bad:?}");
        }
        // The size is a query parameter, whatever it holds.
        assert_eq!(
            url("x", Some("a&b=c")).unwrap(),
            "https://files.messenger.yandex.ru/file_shortterm/x?size=a%26b%3Dc"
        );
    }

    #[tokio::test]
    async fn a_file_over_its_limit_is_refused() {
        let second = Duration::from_secs(1);
        let declared = vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n".to_vec(),
            vec![b'x'; 100],
        ];
        let url = serve_once(declared, Duration::ZERO).await;
        assert!(matches!(
            client().fetch(url.parse().unwrap(), 50, second).await,
            Err(WebError::TooLarge(50))
        ));
        // Without a length the download stops once it passes the limit.
        let url = serve_once(chunked(2), Duration::ZERO).await;
        assert!(matches!(
            client().fetch(url.parse().unwrap(), 50, second).await,
            Err(WebError::TooLarge(50))
        ));
        let url = serve_once(chunked(2), Duration::ZERO).await;
        let (bytes, kind) = client().fetch(url.parse().unwrap(), 80, second).await.unwrap();
        assert_eq!((bytes.len(), kind.as_str()), (80, "image/png"));
    }

    #[tokio::test]
    async fn a_stalled_download_fails() {
        let short = Duration::from_millis(300);
        let head = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n12345".to_vec();
        let url = serve_once(vec![head], Duration::from_secs(10)).await;
        assert!(matches!(
            client().fetch(url.parse().unwrap(), 100, short).await,
            Err(WebError::Stalled)
        ));
        let url = serve_once(vec![], Duration::from_secs(10)).await;
        assert!(matches!(
            client().fetch(url.parse().unwrap(), 100, short).await,
            Err(WebError::Stalled)
        ));
    }

    #[test]
    fn disk_refusals_are_told_apart() {
        use serde_json::json;
        let refusal = |v: Value| disk_refusal(&v);
        assert!(matches!(
            refusal(json!({"data": {"source": "disk", "code": "507"}})),
            Some(WebError::DiskFull)
        ));
        assert!(matches!(
            refusal(json!({"data": {"source": "disk", "code": 403}})),
            Some(WebError::DiskFull)
        ));
        assert!(matches!(
            refusal(json!({"data": {"source": "disk", "code": "413"}})),
            Some(WebError::DiskTooLarge)
        ));
        assert!(refusal(json!({"data": {"source": "disk", "code": "500"}})).is_none());
        assert!(refusal(json!({"data": {"code": "507"}})).is_none());
        assert!(matches!(
            disk_error(StatusCode::INSUFFICIENT_STORAGE),
            WebError::DiskFull
        ));
        assert!(matches!(
            disk_error(StatusCode::PAYLOAD_TOO_LARGE),
            WebError::DiskTooLarge
        ));
        assert!(disk_retry(&disk_error(StatusCode::BAD_GATEWAY)));
        assert!(!disk_retry(&disk_error(StatusCode::NOT_FOUND)));
        assert!(!disk_retry(&WebError::DiskFull));
    }

    /// What the fake Yandex Disk got: the method, `Content-Range` and the body bytes it read.
    type DiskLog = Arc<Mutex<Vec<(String, Option<String>, Vec<u8>)>>>;

    /// One answer of the fake Yandex Disk.
    enum DiskStep {
        /// Read the whole request, then answer this.
        Reply(&'static str),
        /// Read this many bytes of the body, then drop the connection.
        Cut(usize),
        /// Read this many bytes of the body, then hold the connection without a word.
        Hang(usize),
    }

    /// A fake Yandex Disk on a local port: each request comes on its own connection and gets
    /// the next of `steps`.
    async fn fake_disk(steps: Vec<DiskStep>) -> (String, DiskLog) {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/upload-target/x", listener.local_addr().unwrap());
        let log = DiskLog::default();
        let seen = log.clone();
        tokio::spawn(async move {
            for step in steps {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let mut buf = vec![0; 65536];
                let head_end = loop {
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
                let header = |name: &str| {
                    head.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
                    })
                };
                let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
                let wanted = match step {
                    DiskStep::Cut(n) | DiskStep::Hang(n) => n.min(length),
                    DiskStep::Reply(_) => length,
                };
                let mut body = data[head_end..].to_vec();
                while body.len() < wanted {
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&buf[..n]);
                }
                body.truncate(length);
                let method = head.split(' ').next().unwrap_or_default().to_owned();
                seen.lock().unwrap().push((method, header("content-range"), body));
                match step {
                    DiskStep::Reply(answer) => {
                        let _ = s.write_all(answer.as_bytes()).await;
                    }
                    DiskStep::Hang(_) => {
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            drop(s);
                        });
                    }
                    DiskStep::Cut(_) => {}
                }
            }
        });
        (url, log)
    }

    const FAST_DISK: DiskTiming = DiskTiming {
        processing: Duration::from_secs(5),
        poll: Duration::from_millis(10),
        resume_pauses: &[Duration::from_millis(10); 3],
        stall: Duration::from_millis(300),
    };

    const ACCEPTED: &str = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const CREATED: &str =
        "HTTP/1.1 201 Created\r\nLocation: disk:/x.bin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    /// The file goes whole with its range, then empty `PUT`s ask until the Disk has taken it in.
    #[tokio::test]
    async fn a_file_goes_to_yandex_disk_and_is_waited_for() {
        let size = UPLOAD_CHUNK * 3 + 5;
        let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let (url, log) = fake_disk(vec![
            DiskStep::Reply(ACCEPTED),
            DiskStep::Reply(ACCEPTED),
            DiskStep::Reply(CREATED),
        ])
        .await;
        let last = Arc::new(Mutex::new(0));
        let seen = last.clone();
        let file = DiskFile::Memory(bytes.clone().into());
        let progress = Arc::new(move |n| *seen.lock().unwrap() = n);
        let location = client()
            .disk_put_timed(&url, &file, size as u64, false, progress, FAST_DISK)
            .await
            .unwrap();
        assert_eq!(location, "disk:/x.bin");
        assert_eq!(*last.lock().unwrap(), size as u64);
        let log = log.lock().unwrap();
        let range = format!("bytes 0-{}/{size}", size - 1);
        assert_eq!((log[0].0.as_str(), log[0].1.as_deref()), ("PUT", Some(range.as_str())));
        assert_eq!(log[0].2, bytes);
        for ask in &log[1..] {
            assert_eq!((ask.0.as_str(), ask.1.as_deref(), ask.2.len()), ("PUT", None, 0));
        }
        assert_eq!(log.len(), 3);
    }

    /// A broken upload asks the Disk how much it has and sends the rest of the file from there.
    #[tokio::test]
    async fn a_broken_upload_to_yandex_disk_goes_on() {
        let size = 300_000;
        let bytes: Vec<u8> = (0..size).map(|i| (i % 253) as u8).collect();
        let path = std::env::temp_dir().join(format!("mostik-disk-{}.bin", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let (url, log) = fake_disk(vec![
            DiskStep::Cut(1000),
            DiskStep::Reply("HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n"),
            DiskStep::Reply(CREATED),
        ])
        .await;
        let file = DiskFile::Path(path.clone());
        let location = client()
            .disk_put_timed(&url, &file, size as u64, false, Arc::new(|_| {}), FAST_DISK)
            .await
            .unwrap();
        assert_eq!(location, "disk:/x.bin");
        let log = log.lock().unwrap();
        assert_eq!(log[1].0, "HEAD");
        let range = format!("bytes 100000-{}/{size}", size - 1);
        assert_eq!((log[2].0.as_str(), log[2].1.as_deref()), ("PUT", Some(range.as_str())));
        assert_eq!(log[2].2, bytes[100_000..]);
        let _ = std::fs::remove_file(&path);
    }

    /// A retry of the sending goes on at the address of the try before: it asks the Disk first
    /// how much it has, and sends only the rest (#65).
    #[tokio::test]
    async fn a_retry_goes_on_where_the_disk_got_to() {
        let size = 2000;
        let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let (url, log) = fake_disk(vec![
            DiskStep::Reply("HTTP/1.1 200 OK\r\nContent-Length: 1500\r\nConnection: close\r\n\r\n"),
            DiskStep::Reply(CREATED),
        ])
        .await;
        let file = DiskFile::Memory(bytes.clone().into());
        let location = client()
            .disk_put_timed(&url, &file, size as u64, true, Arc::new(|_| {}), FAST_DISK)
            .await
            .unwrap();
        assert_eq!(location, "disk:/x.bin");
        let log = log.lock().unwrap();
        assert_eq!(log[0].0, "HEAD");
        assert_eq!(log[1].1.as_deref(), Some("bytes 1500-1999/2000"));
        assert_eq!(log[1].2, bytes[1500..]);
    }

    /// Breaks that each got further do not use up the tries: only the ones in a row that got
    /// nowhere count (#65).
    #[tokio::test]
    async fn breaks_that_get_further_do_not_use_up_the_tries() {
        let size = 10_000;
        let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let has = |n: usize| -> &'static str {
            Box::leak(format!("HTTP/1.1 200 OK\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n").into_boxed_str())
        };
        // Four breaks with three tries: each one after more of the file got there.
        let mut steps = Vec::new();
        for got in [1000, 2000, 3000, 4000] {
            steps.push(DiskStep::Cut(500));
            steps.push(DiskStep::Reply(has(got)));
        }
        steps.push(DiskStep::Reply(CREATED));
        let (url, _) = fake_disk(steps).await;
        let file = DiskFile::Memory(bytes.into());
        let location = client()
            .disk_put_timed(&url, &file, size as u64, false, Arc::new(|_| {}), FAST_DISK)
            .await
            .unwrap();
        assert_eq!(location, "disk:/x.bin");
        // Four breaks in a row that get nowhere are too many.
        let mut steps = Vec::new();
        for _ in 0..4 {
            steps.push(DiskStep::Cut(0));
            steps.push(DiskStep::Reply(has(0)));
        }
        let (url, _) = fake_disk(steps).await;
        let file = DiskFile::Memory(vec![1; 10].into());
        let result = client()
            .disk_put_timed(&url, &file, 10, false, Arc::new(|_| {}), FAST_DISK)
            .await;
        assert!(result.is_err());
    }

    /// A `PUT` that stops taking bytes before the end of the file (the connection died on the
    /// way, and nothing tells it) is broken off and goes on like a broken one, telling `progress`
    /// meanwhile that the upload is alive (#65).
    #[tokio::test]
    async fn a_put_that_stalls_goes_on() {
        // More than the socket buffers take in: the rest of the body stays unsent.
        let size = 24 * 1024 * 1024;
        let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let (url, log) = fake_disk(vec![
            DiskStep::Hang(1000),
            DiskStep::Reply("HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n"),
            DiskStep::Reply(CREATED),
        ])
        .await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let file = DiskFile::Memory(bytes.clone().into());
        let started = std::time::Instant::now();
        let location = client()
            .disk_put_timed(
                &url,
                &file,
                size as u64,
                false,
                Arc::new(move |n| seen.lock().unwrap().push(n)),
                FAST_DISK,
            )
            .await
            .unwrap();
        assert_eq!(location, "disk:/x.bin");
        // Not after the fake Disk lets the stalled connection go, 30 s on.
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        let log = log.lock().unwrap();
        assert_eq!(log[1].0, "HEAD");
        let range = format!("bytes 1000-{}/{size}", size - 1);
        assert_eq!(log[2].1.as_deref(), Some(range.as_str()));
        assert_eq!(log[2].2, bytes[1000..]);
        // The break said the upload was alive, at where the Disk was known to be.
        assert!(calls.lock().unwrap().contains(&0));
    }

    /// A full Disk is told at once, without going on.
    #[tokio::test]
    async fn a_full_yandex_disk_stops_the_upload() {
        let (url, log) = fake_disk(vec![DiskStep::Reply(
            "HTTP/1.1 507 Insufficient Storage\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )])
        .await;
        let file = DiskFile::Memory(vec![1; 10].into());
        let result = client()
            .disk_put_timed(&url, &file, 10, false, Arc::new(|_| {}), FAST_DISK)
            .await;
        assert!(matches!(result, Err(WebError::DiskFull)));
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn transfer_time_grows_with_the_size() {
        assert_eq!(transfer_timeout(0), Duration::from_secs(60));
        assert_eq!(transfer_timeout(60 * SLOWEST_TRANSFER), Duration::from_secs(120));
        assert_eq!(transfer_timeout(u64::MAX), MAX_TRANSFER_TIME);
    }

    #[test]
    fn percent_coding() {
        assert_eq!(
            encode_uri_component("Отчёт (1)!.pdf"),
            "%D0%9E%D1%82%D1%87%D1%91%D1%82%20(1)!.pdf"
        );
        assert_eq!(encode_uri_component("a/b?c#d&e*f'~"), "a%2Fb%3Fc%23d%26e%2Af'~");
        assert_eq!(
            encode_uri_component("<b>a b\n#&</b>"),
            "%3Cb%3Ea%20b%0A%23%26%3C%2Fb%3E"
        );
        assert_eq!(percent_decode("disk%2Fabc-1"), "disk/abc-1");
        assert_eq!(percent_decode("%D1%84%D0%B0%D0%B9%D0%BB"), "файл");
        assert_eq!(percent_decode("bad%2"), "bad%2");
        assert_eq!(percent_decode("%zz%+5"), "%zz%+5");
        assert_eq!(percent_decode("%FF"), "\u{FFFD}");
        assert_eq!(percent_decode("plain"), "plain");
        let name = "Отчёт за май (итог) #2 100%.docx";
        assert_eq!(percent_decode(&encode_uri_component(name)), name);
    }

    #[test]
    fn parses_csrf_variants() {
        assert_eq!(parse_csrf(r#"{"token":"t1"}"#).as_deref(), Some("t1"));
        assert_eq!(parse_csrf(r#"{"data":{"csrf_token":"t2"}}"#).as_deref(), Some("t2"));
        assert_eq!(parse_csrf("t3:1700000000").as_deref(), Some("t3:1700000000"));
        assert_eq!(parse_csrf("<html> login </html>"), None);
    }
}
