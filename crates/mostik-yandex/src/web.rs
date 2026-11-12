//! HTTP side of the client: the web client page, CSRF and the `/api/` RPC.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue, LOCATION, ORIGIN, REFERER, SET_COOKIE, USER_AGENT};
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
pub fn parse_page_params(html: &str) -> Result<PageParams, WebError> {
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
pub fn parse_csrf(body: &str) -> Option<String> {
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
    pub async fn csrf_token(&self) -> Result<String, WebError> {
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
        self.fetch(&file_url(file_id, size), limit, STALL_TIMEOUT).await
    }

    /// Writes a file of the Messenger storage into `out` as it arrives, whatever its size.
    /// Returns the number of bytes written.
    pub async fn download(&self, file_id: &str, out: &mut (impl AsyncWrite + Unpin)) -> Result<u64, WebError> {
        let mut download = self.open(&file_url(file_id, None), u64::MAX, STALL_TIMEOUT).await?;
        while let Some(chunk) = download.next().await? {
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        Ok(download.got)
    }

    /// [`WebClient::fetch_file`] from `url`, with the given wait for data.
    async fn fetch(&self, url: &str, limit: u64, stall: Duration) -> Result<(Vec<u8>, String), WebError> {
        let mut download = self.open(url, limit, stall).await?;
        let mut bytes = Vec::with_capacity(download.expected as usize);
        while let Some(chunk) = download.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        Ok((bytes, download.kind))
    }

    /// Requests a file; refuses it at once when its `Content-Length` is over `limit`.
    async fn open(&self, url: &str, limit: u64, stall: Duration) -> Result<Download, WebError> {
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
    /// `message_id` is the UUID of the future message (also its `PayloadId`). Returns `data.data` of the response.
    pub async fn upload_file(
        &mut self,
        chat_id: &str,
        message_id: &str,
        filename: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<Value, WebError> {
        // Like `encodeURI` in the web client: the chat ID goes in with its slashes as is,
        // only the file name is encoded (and a `/` in it is escaped so it cannot break the path).
        let enc = |s: &str| -> String {
            s.bytes()
                .map(|b| match b {
                    b'A'..=b'Z'
                    | b'a'..=b'z'
                    | b'0'..=b'9'
                    | b'-'
                    | b'_'
                    | b'.'
                    | b'~'
                    | b'('
                    | b')'
                    | b'!'
                    | b'\'' => (b as char).to_string(),
                    _ => format!("%{b:02X}"),
                })
                .collect()
        };
        let url = format!(
            "https://{}/media_upload/{chat_id}/{message_id}/{}",
            FILES_HOST,
            enc(filename)
        );
        let bytes = bytes::Bytes::from(bytes);
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
            let resp = self
                .http
                .post(&url)
                .headers(headers)
                .body(bytes.clone())
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
        Ok(v.get("data").cloned().unwrap_or(v))
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

/// Address of a file or preview of the Messenger storage: `file_shortterm/<Id2>[?size=…]`.
fn file_url(file_id: &str, size: Option<&str>) -> String {
    let mut url = format!("https://{FILES_HOST}/file_shortterm/{file_id}");
    if let Some(size) = size {
        url.push_str("?size=");
        url.push_str(size);
    }
    url
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

    #[tokio::test]
    async fn a_file_over_its_limit_is_refused() {
        let second = Duration::from_secs(1);
        let declared = vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n".to_vec(),
            vec![b'x'; 100],
        ];
        let url = serve_once(declared, Duration::ZERO).await;
        assert!(matches!(
            client().fetch(&url, 50, second).await,
            Err(WebError::TooLarge(50))
        ));
        // Without a length the download stops once it passes the limit.
        let url = serve_once(chunked(2), Duration::ZERO).await;
        assert!(matches!(
            client().fetch(&url, 50, second).await,
            Err(WebError::TooLarge(50))
        ));
        let url = serve_once(chunked(2), Duration::ZERO).await;
        let (bytes, kind) = client().fetch(&url, 80, second).await.unwrap();
        assert_eq!((bytes.len(), kind.as_str()), (80, "image/png"));
    }

    #[tokio::test]
    async fn a_stalled_download_fails() {
        let short = Duration::from_millis(300);
        let head = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n12345".to_vec();
        let url = serve_once(vec![head], Duration::from_secs(10)).await;
        assert!(matches!(client().fetch(&url, 100, short).await, Err(WebError::Stalled)));
        let url = serve_once(vec![], Duration::from_secs(10)).await;
        assert!(matches!(client().fetch(&url, 100, short).await, Err(WebError::Stalled)));
    }

    #[test]
    fn transfer_time_grows_with_the_size() {
        assert_eq!(transfer_timeout(0), Duration::from_secs(60));
        assert_eq!(transfer_timeout(60 * SLOWEST_TRANSFER), Duration::from_secs(120));
        assert_eq!(transfer_timeout(u64::MAX), MAX_TRANSFER_TIME);
    }

    #[test]
    fn parses_csrf_variants() {
        assert_eq!(parse_csrf(r#"{"token":"t1"}"#).as_deref(), Some("t1"));
        assert_eq!(parse_csrf(r#"{"data":{"csrf_token":"t2"}}"#).as_deref(), Some("t2"));
        assert_eq!(parse_csrf("t3:1700000000").as_deref(), Some("t3:1700000000"));
        assert_eq!(parse_csrf("<html> login </html>"), None);
    }
}
