//! Xiva WebSocket: a background reader, responses matched to requests by `seq`, pushes go to an event handler.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::endpoints::{WEB_ORIGIN, xiva_ws_url};
use crate::session::CookieJar;
use crate::xiva::{self, Frame};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, thiserror::Error)]
pub enum XivaError {
    #[error("WebSocket: {0}")]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("frame: {0}")]
    Frame(#[from] xiva::FrameError),
    #[error("no response to {0} within {1} s")]
    Timeout(String, u64),
    #[error("Telemost's response to {0} could not be read: {1}")]
    BadResponse(String, String),
    #[error("connection to Telemost closed")]
    Closed,
}

/// Everything received over the socket except responses to our requests.
#[derive(Debug, Clone)]
pub enum Incoming {
    /// `messenger-prod / delivery` push: the body is `{ClientMessage, ServerMessageInfo, …}`.
    Push { event: String, body: Value },
    /// Xiva service text frame (subscription, ping).
    Text(Value),
    /// The connection is closed.
    Closed,
}

type Pending = Arc<StdMutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// The key of the stand-in body a waiter gets when its response did not decode.
const UNDECODED: &str = "__undecoded";

/// How often Xiva pings until its first ping says (`server-interval-sec`).
const DEFAULT_PING_INTERVAL_MS: u64 = 60_000;

/// Finds a connection that died without closing (sleep, a network change, a dropped NAT
/// mapping): requests on it would wait for their timeouts and pushes would stop for good.
/// Xiva sends a text ping every `server-interval-sec`; a connection that has been silent for
/// one and a half of that is dead, and the reader stops, which reports it closed. (WebSocket
/// pings do not help: Xiva answers them seconds late or not at all.)
#[derive(Clone)]
struct Liveness {
    closed: Arc<AtomicBool>,
    /// Milliseconds since the Unix epoch of the last frame received.
    last_frame: Arc<AtomicU64>,
    ping_interval_ms: Arc<AtomicU64>,
    kill: Arc<Notify>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Liveness {
    /// Drops the connection if it has missed Xiva's ping.
    fn check(&self) {
        let interval = self.ping_interval_ms.load(Ordering::SeqCst);
        let silent = now_ms().saturating_sub(self.last_frame.load(Ordering::SeqCst));
        if !self.closed.load(Ordering::SeqCst) && silent > interval + interval / 2 {
            eprintln!(
                "xiva: nothing received for {} s, the connection is dropped",
                silent / 1000
            );
            self.kill.notify_one();
        }
    }

    /// Checks the connection every few seconds until it closes.
    fn watch(self) {
        tokio::spawn(async move {
            while !self.closed.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(10)).await;
                self.check();
            }
        });
    }
}

/// Xiva connection: one writer, one background reader task.
pub struct XivaClient {
    writer: Arc<Mutex<SplitSink<Ws, WsMessage>>>,
    pending: Pending,
    seq: AtomicU64,
    closed: Arc<AtomicBool>,
    subscription_id: Arc<StdMutex<Option<String>>>,
    /// The reader task; aborting it drops the socket.
    reader: tokio::task::AbortHandle,
    liveness: Liveness,
}

impl Drop for XivaClient {
    /// Closes the socket: without this the reader task would keep the connection alive and keep
    /// feeding `on_event` after the client is gone. No `Incoming::Closed` is sent for a dropped client.
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.reader.abort();
    }
}

impl XivaClient {
    /// Opens the socket and starts the reader; every push and service frame goes to `on_event`.
    pub async fn connect<F>(jar: &CookieJar, user_agent: &str, puid: &str, on_event: F) -> Result<Self, XivaError>
    where
        F: Fn(Incoming) + Send + Sync + 'static,
    {
        let session = uuid::Uuid::new_v4().to_string();
        let mut req = xiva_ws_url(&session, puid).into_client_request()?;
        let h = req.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&jar.header_for("ws.push.yandex.ru")) {
            h.insert("Cookie", v);
        }
        if let Ok(v) = HeaderValue::from_str(user_agent) {
            h.insert("User-Agent", v);
        }
        h.insert("Origin", HeaderValue::from_static(WEB_ORIGIN));
        let (ws, _) = connect_async(req).await?;
        let (sink, mut stream) = ws.split();
        let writer = Arc::new(Mutex::new(sink));
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let subscription_id: Arc<StdMutex<Option<String>>> = Arc::default();

        let liveness = Liveness {
            closed: closed.clone(),
            last_frame: Arc::new(AtomicU64::new(now_ms())),
            ping_interval_ms: Arc::new(AtomicU64::new(DEFAULT_PING_INTERVAL_MS)),
            kill: Arc::default(),
        };
        let (w, p, c, sub, live) = (
            writer.clone(),
            pending.clone(),
            closed.clone(),
            subscription_id.clone(),
            liveness.clone(),
        );
        let reader = tokio::spawn(async move {
            loop {
                let msg = tokio::select! {
                    msg = stream.next() => msg,
                    () = live.kill.notified() => break,
                };
                let Some(Ok(msg)) = msg else { break };
                live.last_frame.store(now_ms(), Ordering::SeqCst);
                match msg {
                    WsMessage::Binary(data) => match xiva::decode(&data) {
                        Ok(Frame::Rpc { seq, body, .. }) => {
                            if let Some(tx) = p.lock().ok().and_then(|mut m| m.remove(&seq)) {
                                let _ = tx.send(body);
                            }
                        }
                        Ok(Frame::Push { event, body, .. }) => on_event(Incoming::Push { event, body }),
                        // A response that does not decode still answers its request, so the caller
                        // does not wait for the timeout. (Some pushes carry protobuf, not JSON:
                        // those are not read yet.)
                        Err(e) => {
                            if let Some(tx) = xiva::rpc_seq(&data).and_then(|seq| p.lock().ok()?.remove(&seq)) {
                                eprintln!("xiva: response of {} bytes not decoded: {e}", data.len());
                                let _ = tx.send(serde_json::json!({ UNDECODED: e.to_string() }));
                            }
                        }
                    },
                    WsMessage::Text(t) => {
                        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                        if let Some(sec) = v.get("server-interval-sec").and_then(Value::as_u64)
                            && sec > 0
                        {
                            live.ping_interval_ms.store(sec * 1000, Ordering::SeqCst);
                        }
                        if let Some(id) = v.get("subscription-id").and_then(Value::as_str)
                            && let Ok(mut s) = sub.lock()
                        {
                            *s = Some(id.to_owned());
                        }
                        on_event(Incoming::Text(v));
                    }
                    WsMessage::Ping(payload) => {
                        let _ = w.lock().await.send(WsMessage::Pong(payload)).await;
                    }
                    WsMessage::Close(_) => break,
                    _ => {}
                }
            }
            c.store(true, Ordering::SeqCst);
            // Release the waiters: their `call` returns `Closed`.
            if let Ok(mut m) = p.lock() {
                m.clear();
            }
            on_event(Incoming::Closed);
        });

        liveness.clone().watch();
        Ok(Self {
            writer,
            pending,
            seq: AtomicU64::new(0),
            closed,
            subscription_id,
            reader: reader.abort_handle(),
            liveness,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Xiva subscription ID from the service frame; `push` needs it (`ClientTransportId`).
    pub fn subscription_id(&self) -> Option<String> {
        self.subscription_id.lock().ok().and_then(|s| s.clone())
    }

    /// Waits for the subscription ID that Xiva sends right after connecting.
    pub async fn wait_subscription(&self, timeout: Duration) -> Option<String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(id) = self.subscription_id() {
                return Some(id);
            }
            if self.is_closed() || tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Sends a request and waits for the response with the same `seq`.
    pub async fn call(&self, method: &str, body: &impl Serialize, timeout_s: u64) -> Result<Value, XivaError> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut m) = self.pending.lock() {
            m.insert(seq, tx);
        }
        // Checked after registering: the reader marks the socket closed before it drops the waiters,
        // so either the check sees the close or the waiter is dropped and `rx` fails at once.
        if self.is_closed() {
            self.forget(seq);
            return Err(XivaError::Closed);
        }
        let sent = match xiva::encode_rpc(seq, method, body) {
            Ok(bytes) => self
                .writer
                .lock()
                .await
                .send(WsMessage::Binary(bytes.into()))
                .await
                .map_err(XivaError::from),
            Err(e) => Err(e.into()),
        };
        if let Err(e) = sent {
            self.forget(seq);
            return Err(e);
        }
        match tokio::time::timeout(Duration::from_secs(timeout_s), rx).await {
            Ok(Ok(v)) => match v.get(UNDECODED).and_then(Value::as_str) {
                Some(e) => Err(XivaError::BadResponse(method.to_owned(), e.to_owned())),
                None => Ok(v),
            },
            Ok(Err(_)) => Err(XivaError::Closed),
            Err(_) => {
                self.forget(seq);
                // Maybe the connection is dead: then it is dropped and made again.
                self.liveness.check();
                Err(XivaError::Timeout(method.to_owned(), timeout_s))
            }
        }
    }

    fn forget(&self, seq: u64) {
        if let Ok(mut m) = self.pending.lock() {
            m.remove(&seq);
        }
    }
}
