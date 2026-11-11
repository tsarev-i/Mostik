//! HTTP side of the client: the web client page, CSRF and the `/api/` RPC.

use std::sync::{Arc, Mutex};

use reqwest::StatusCode;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue, LOCATION, ORIGIN, REFERER, SET_COOKIE, USER_AGENT};
use serde_json::Value;

use crate::endpoints::{API_URL, CSRF_URL, WEB_ORIGIN};
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
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
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
    pub fn new(jar: CookieJar, user_agent: String) -> Result<Self, WebError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        let page_http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
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

    pub fn user_agent(&self) -> &str {
        &self.user_agent
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

    /// File or preview from the Messenger storage: `file_shortterm/<Id2>[?size=…]`.
    /// Returns the content and its `Content-Type`.
    pub async fn fetch_file(&self, file_id: &str, size: Option<&str>) -> Result<(Vec<u8>, String), WebError> {
        let mut url = format!("https://{}/file_shortterm/{file_id}", crate::endpoints::FILES_HOST);
        if let Some(size) = size {
            url.push_str("?size=");
            url.push_str(size);
        }
        let resp = self
            .http
            .get(url)
            .headers(self.headers(crate::endpoints::FILES_HOST))
            .send()
            .await?;
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(WebError::Unauthorized);
        }
        if !status.is_success() {
            return Err(WebError::BadResponse(format!("file: HTTP {status}")));
        }
        let kind = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        Ok((resp.bytes().await?.to_vec(), kind))
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
            crate::endpoints::FILES_HOST,
            enc(filename)
        );
        let bytes = bytes::Bytes::from(bytes);
        let mut retried = false;
        let (status, body) = loop {
            let csrf = self.csrf_token().await?;
            let mut headers = self.headers(crate::endpoints::FILES_HOST);
            if let Ok(v) = HeaderValue::from_str(&csrf) {
                headers.insert("X-CSRF-TOKEN", v);
            }
            if let Ok(v) = HeaderValue::from_str(content_type) {
                headers.insert(reqwest::header::CONTENT_TYPE, v);
            }
            let resp = self.http.post(&url).headers(headers).body(bytes.clone()).send().await?;
            let status = resp.status();
            let body = resp.text().await?;
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

    /// `POST /api/` call in the web client format. Returns the whole response JSON.
    pub async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, WebError> {
        let request = RpcRequest::new(method, params).form_value()?;
        let mut retried = false;
        loop {
            let csrf = self.csrf_token().await?;
            let form = reqwest::multipart::Form::new().text(FORM_FIELD, request.clone());
            let mut headers = self.headers("messenger.360.yandex.ru");
            if let Ok(v) = HeaderValue::from_str(&csrf) {
                headers.insert("X-CSRF-TOKEN", v);
            }
            headers.insert(
                "X-Request-Attempt",
                HeaderValue::from_static(if retried { "1" } else { "0" }),
            );
            if let Some(id) = self.page.as_ref().and_then(|p| p.service_id.as_deref())
                && let Ok(v) = HeaderValue::from_str(id)
            {
                headers.insert("X-Origin-Service-ID", v);
            }
            let resp = self.http.post(API_URL).headers(headers).multipart(form).send().await?;
            let status = resp.status();
            let body = resp.text().await?;
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
            return parsed.map_err(|_| WebError::BadResponse(format!("{method}: HTTP {status}, not JSON")));
        }
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

    #[test]
    fn parses_csrf_variants() {
        assert_eq!(parse_csrf(r#"{"token":"t1"}"#).as_deref(), Some("t1"));
        assert_eq!(parse_csrf(r#"{"data":{"csrf_token":"t2"}}"#).as_deref(), Some("t2"));
        assert_eq!(parse_csrf("t3:1700000000").as_deref(), Some("t3:1700000000"));
        assert_eq!(parse_csrf("<html> login </html>"), None);
    }
}
