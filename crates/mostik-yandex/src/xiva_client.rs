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
use tokio::sync::{Mutex, oneshot};
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

/// Xiva connection: one writer, one background reader task.
pub struct XivaClient {
    writer: Arc<Mutex<SplitSink<Ws, WsMessage>>>,
    pending: Pending,
    seq: AtomicU64,
    closed: Arc<AtomicBool>,
    subscription_id: Arc<StdMutex<Option<String>>>,
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

        let (w, p, c, sub) = (writer.clone(), pending.clone(), closed.clone(), subscription_id.clone());
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                let Ok(msg) = msg else { break };
                match msg {
                    WsMessage::Binary(data) => match xiva::decode(&data) {
                        Ok(Frame::Rpc { seq, body, .. }) => {
                            if let Some(tx) = p.lock().ok().and_then(|mut m| m.remove(&seq)) {
                                let _ = tx.send(body);
                            }
                        }
                        Ok(Frame::Push { event, body, .. }) => on_event(Incoming::Push { event, body }),
                        Err(_) => {}
                    },
                    WsMessage::Text(t) => {
                        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
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

        Ok(Self {
            writer,
            pending,
            seq: AtomicU64::new(0),
            closed,
            subscription_id,
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
        if self.is_closed() {
            return Err(XivaError::Closed);
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut m) = self.pending.lock() {
            m.insert(seq, tx);
        }
        let bytes = xiva::encode_rpc(seq, method, body)?;
        self.writer.lock().await.send(WsMessage::Binary(bytes.into())).await?;
        match tokio::time::timeout(Duration::from_secs(timeout_s), rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err(XivaError::Closed),
            Err(_) => {
                if let Ok(mut m) = self.pending.lock() {
                    m.remove(&seq);
                }
                Err(XivaError::Timeout(method.to_owned(), timeout_s))
            }
        }
    }
}
