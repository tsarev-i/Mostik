//! Xiva WebSocket: a background reader, responses matched to requests by `seq`, pushes go to an event handler.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures_util::stream::SplitSink;
use futures_util::{FutureExt, SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, Notify, oneshot};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use crate::endpoints::{WEB_ORIGIN, xiva_ws_url};
use crate::session::CookieJar;
use crate::xiva::{self, Frame};

/// What the socket runs over: TLS to Xiva, or an in-memory pipe in the tests.
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

type Ws = WebSocketStream<Box<dyn Io>>;

#[derive(Debug, thiserror::Error)]
pub enum XivaError {
    #[error("WebSocket: {0}")]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("{0}")]
    Net(#[from] crate::net::NetError),
    #[error("no connection to Telemost within {0} s")]
    ConnectTimeout(u64),
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
    /// (Its protobuf twins are skipped, see [`xiva`].)
    Push { event: String, body: Value },
    /// Xiva service text frame (subscription, ping).
    Text(Value),
    /// The connection is closed.
    Closed,
}

/// Waiters for responses by `seq`: a response that did not decode comes as the error text.
type Pending = Arc<StdMutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// How long opening the socket may take: the proxy, TLS and the WebSocket handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

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
        let host = req.uri().host().unwrap_or_default().to_owned();
        let port = req.uri().port_u16().unwrap_or(443);
        let open = async {
            let tls = crate::net::connect_tls(&host, port).await?;
            let (ws, _) = tokio_tungstenite::client_async(req, Box::new(tls) as Box<dyn Io>).await?;
            Ok::<_, XivaError>(ws)
        };
        let ws = tokio::time::timeout(CONNECT_TIMEOUT, open)
            .await
            .map_err(|_| XivaError::ConnectTimeout(CONNECT_TIMEOUT.as_secs()))??;
        Ok(Self::start(ws, on_event))
    }

    /// Starts the reader of an open socket.
    fn start<F>(ws: Ws, on_event: F) -> Self
    where
        F: Fn(Incoming) + Send + Sync + 'static,
    {
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
            // Why the connection ended, for the log: Telemost drops it now and then for no
            // visible reason.
            let frames = async {
                loop {
                    let msg = tokio::select! {
                        msg = stream.next() => msg,
                        () = live.kill.notified() => break "no ping from Xiva".to_owned(),
                    };
                    let msg = match msg {
                        Some(Ok(msg)) => msg,
                        Some(Err(e)) => break format!("read error: {e}"),
                        None => break "the stream ended".to_owned(),
                    };
                    live.last_frame.store(now_ms(), Ordering::SeqCst);
                    match msg {
                        WsMessage::Binary(data) => match xiva::decode(&data) {
                            Ok(Frame::Rpc { seq, body, .. }) => {
                                if crate::diag::traffic_logging() {
                                    crate::diag::traffic_log(
                                        serde_json::json!({ "dir": "in", "via": "ws", "seq": seq, "body": body }),
                                    );
                                }
                                if let Some(tx) = p.lock().ok().and_then(|mut m| m.remove(&seq)) {
                                    let _ = tx.send(Ok(body));
                                }
                            }
                            Ok(Frame::Push { event, body, .. }) => on_event(Incoming::Push { event, body }),
                            Ok(Frame::ProtobufTwin { .. }) => {}
                            // A response that does not decode still answers its request, so the
                            // caller does not wait for the timeout.
                            Err(e) => match xiva::rpc_seq(&data) {
                                Some(seq) => {
                                    crate::diag::traffic_log(serde_json::json!({
                                        "dir": "in", "via": "ws", "seq": seq, "error": e.to_string(), "hex": crate::diag::hex(&data)
                                    }));
                                    if let Some(tx) = p.lock().ok().and_then(|mut m| m.remove(&seq)) {
                                        eprintln!("xiva: response of {} bytes not decoded: {e}", data.len());
                                        let _ = tx.send(Err(e.to_string()));
                                    }
                                }
                                None => eprintln!("xiva: a frame of {} bytes not decoded: {e}", data.len()),
                            },
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
                        WsMessage::Close(frame) => {
                            break match frame {
                                Some(f) => format!("close frame {}: {}", u16::from(f.code), f.reason),
                                None => "close frame without a code".to_owned(),
                            };
                        }
                        _ => {}
                    }
                }
            };
            // A panic in `on_event` ends the connection the way a network error does. Uncaught, it
            // would end the reader and leave the socket looking alive with nobody reading it; and
            // the handler's state may be half-changed (a poisoned lock), which the reconnection
            // starts afresh.
            let reason = match AssertUnwindSafe(frames).catch_unwind().await {
                Ok(reason) => reason,
                Err(panic) => format!("the push handler panicked: {}", panic_text(panic.as_ref())),
            };
            eprintln!("xiva: the connection closed: {reason}");
            crate::diag::traffic_log(serde_json::json!({ "dir": "in", "via": "ws", "closed": reason }));
            c.store(true, Ordering::SeqCst);
            // Release the waiters: their `call` returns `Closed`.
            if let Ok(mut m) = p.lock() {
                m.clear();
            }
            if std::panic::catch_unwind(AssertUnwindSafe(|| on_event(Incoming::Closed))).is_err() {
                eprintln!("xiva: the push handler panicked on the close");
            }
        });

        liveness.clone().watch();
        Self {
            writer,
            pending,
            seq: AtomicU64::new(0),
            closed,
            subscription_id,
            reader: reader.abort_handle(),
            liveness,
        }
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
        if crate::diag::traffic_logging() {
            crate::diag::traffic_log(serde_json::json!({
                "dir": "out", "via": "ws", "seq": seq, "method": method, "body": serde_json::to_value(body).ok()
            }));
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
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(XivaError::BadResponse(method.to_owned(), e)),
            Ok(Err(_)) => Err(XivaError::Closed),
            Err(_) => {
                crate::diag::traffic_log(
                    serde_json::json!({ "dir": "in", "via": "ws", "seq": seq, "timeout": method }),
                );
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

/// The message of a caught panic.
fn panic_text(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xiva::Meta;
    use tokio_tungstenite::tungstenite::protocol::Role;

    /// A client on one end of an in-memory pipe and the server's end of it.
    async fn pair<F>(on_event: F) -> (XivaClient, WebSocketStream<tokio::io::DuplexStream>)
    where
        F: Fn(Incoming) + Send + Sync + 'static,
    {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let client = WebSocketStream::from_raw_socket(Box::new(client) as Box<dyn Io>, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        (XivaClient::start(client, on_event), server)
    }

    fn push(version: u32, body: &[u8]) -> WsMessage {
        let meta = Meta {
            version,
            aux: 1,
            reserved: 0,
        };
        WsMessage::Binary(xiva::encode_push(meta, body).into())
    }

    /// What the handler got, in order: `push`, `text` or `closed`.
    fn recorder() -> (Arc<StdMutex<Vec<String>>>, impl Fn(Incoming) + Send + Sync + 'static) {
        let got: Arc<StdMutex<Vec<String>>> = Arc::default();
        let record = got.clone();
        let handler = move |incoming: Incoming| {
            let kind = match incoming {
                Incoming::Push { body, .. } => format!("push {}", body["n"]),
                Incoming::Text(_) => "text".to_owned(),
                Incoming::Closed => "closed".to_owned(),
            };
            record.lock().unwrap().push(kind);
        };
        (got, handler)
    }

    async fn until(what: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !what() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("timed out");
    }

    #[tokio::test]
    async fn protobuf_twins_are_skipped() {
        let (got, handler) = recorder();
        let (_xiva, mut server) = pair(handler).await;
        server.send(push(4, &[0x0a, 0x01, b'x'])).await.unwrap();
        server.send(push(5, br#"{"n":1}"#)).await.unwrap();
        server.close(None).await.unwrap();
        until(|| got.lock().unwrap().len() == 2).await;
        assert_eq!(*got.lock().unwrap(), ["push 1", "closed"]);
    }

    #[tokio::test]
    async fn a_panicking_handler_closes_the_connection() {
        let (got, record) = recorder();
        let handler = move |incoming: Incoming| {
            if matches!(&incoming, Incoming::Push { body, .. } if body["n"] == 2) {
                panic!("a bug in the handler");
            }
            record(incoming);
        };
        let (xiva, mut server) = pair(handler).await;
        let request = serde_json::json!({});
        let call = xiva.call("history", &request, 5);
        let serve = async {
            server.send(push(5, br#"{"n":1}"#)).await.unwrap();
            server.send(push(5, br#"{"n":2}"#)).await.unwrap();
        };
        let (answer, ()) = tokio::join!(call, serve);
        assert!(matches!(answer, Err(XivaError::Closed)), "{answer:?}");
        assert!(xiva.is_closed());
        assert_eq!(*got.lock().unwrap(), ["push 1", "closed"]);
    }

    #[tokio::test]
    async fn a_response_that_does_not_decode_answers_its_call() {
        let (_, handler) = recorder();
        let (xiva, mut server) = pair(handler).await;
        let serve = async {
            let Some(Ok(WsMessage::Binary(request))) = server.next().await else {
                panic!("no request");
            };
            let mut response = request.to_vec();
            response.truncate(response.len() - 2);
            response.extend_from_slice(b"not json");
            server.send(WsMessage::Binary(response.into())).await.unwrap();
        };
        let request = serde_json::json!({});
        let (answer, ()) = tokio::join!(xiva.call("whoami", &request, 5), serve);
        assert!(matches!(answer, Err(XivaError::BadResponse(m, _)) if m == "whoami"));
        assert!(!xiva.is_closed());
    }
}
