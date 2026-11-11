//! A minimal Chrome DevTools Protocol client over `--remote-debugging-pipe`.
//!
//! Chromium reads commands from fd 3 and writes replies and events to fd 4; each message is
//! a JSON object followed by a NUL byte. A pipe, unlike `--remote-debugging-port`, is not
//! reachable by other local processes, so nobody else can drive the call window.
//!
//! Events (`Page.frameNavigated` now, `Runtime.bindingCalled` for call recording later) come
//! out of a channel that [`Cdp::start`] returns.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, PipeReader, PipeWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// How long to wait for a reply before giving up on a command.
const TIMEOUT: Duration = Duration::from_secs(15);

/// A protocol event: `method`, `params` and the page session it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
}

pub struct Cdp {
    writer: Mutex<PipeWriter>,
    next_id: AtomicU64,
    pending: Pending,
    closed: Arc<AtomicBool>,
}

impl Cdp {
    /// Starts the reader thread. `on_close` runs on that thread once the browser closes its end
    /// of the pipe, that is, when the browser exits; then the event channel closes too.
    pub fn start(
        reader: PipeReader,
        writer: PipeWriter,
        on_close: impl FnOnce() + Send + 'static,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Event>) {
        let (events, events_rx) = mpsc::unbounded_channel();
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let cdp = Arc::new(Self {
            writer: Mutex::new(writer),
            next_id: AtomicU64::new(1),
            pending: pending.clone(),
            closed: closed.clone(),
        });
        std::thread::Builder::new()
            .name("cdp-reader".into())
            .spawn(move || {
                read_loop(BufReader::new(reader), &pending, &events);
                drop(events);
                closed.store(true, Ordering::SeqCst);
                // Whoever still waits gets an error instead of a timeout.
                for (_, tx) in pending.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default() {
                    let _ = tx.send(Err("the call window was closed".into()));
                }
                on_close();
            })
            .expect("could not start the CDP reader thread");
        (cdp, events_rx)
    }

    /// Whether the browser has exited.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Sends a command and waits for its result. `session` addresses a page attached
    /// with `Target.attachToTarget` (`flatten: true`); without it the browser itself answers.
    pub async fn send(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
        if self.is_closed() {
            return Err("the call window was closed".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().map_err(|e| e.to_string())?.insert(id, tx);
        let mut bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        bytes.push(0);
        let written = self
            .writer
            .lock()
            .map_err(|e| e.to_string())
            .and_then(|mut w| w.write_all(&bytes).map_err(|e| e.to_string()));
        if let Err(e) = written {
            self.forget(id);
            return Err(format!("{method}: {e}"));
        }
        match tokio::time::timeout(TIMEOUT, rx).await {
            Ok(Ok(result)) => result.map_err(|e| format!("{method}: {e}")),
            Ok(Err(_)) => Err(format!("{method}: no reply")),
            Err(_) => {
                self.forget(id);
                Err(format!("{method}: timed out"))
            }
        }
    }

    fn forget(&self, id: u64) {
        if let Ok(mut p) = self.pending.lock() {
            p.remove(&id);
        }
    }
}

fn read_loop(mut reader: impl BufRead, pending: &Pending, events: &mpsc::UnboundedSender<Event>) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(0, &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        match parse(&buf) {
            Some(Incoming::Reply(id, result)) => {
                if let Some(tx) = pending.lock().ok().and_then(|mut p| p.remove(&id)) {
                    let _ = tx.send(result);
                }
            }
            // Nobody listens any more: fine, the event is not needed.
            Some(Incoming::Event(e)) => drop(events.send(e)),
            None => {}
        }
    }
}

#[derive(Debug)]
enum Incoming {
    /// Reply to a command: its id and either `result` or the `error` message.
    Reply(u64, Result<Value, String>),
    Event(Event),
}

fn parse(bytes: &[u8]) -> Option<Incoming> {
    let msg: Value = serde_json::from_slice(bytes).ok()?;
    if let Some(id) = msg.get("id").and_then(Value::as_u64) {
        let result = match msg.get("error") {
            Some(e) => Err(e
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| e.to_string(), str::to_owned)),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        return Some(Incoming::Reply(id, result));
    }
    Some(Incoming::Event(Event {
        method: msg.get("method")?.as_str()?.to_owned(),
        params: msg.get("params").cloned().unwrap_or(Value::Null),
        session: msg.get("sessionId").and_then(Value::as_str).map(str::to_owned),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_with_result() {
        let Some(Incoming::Reply(id, r)) = parse(br#"{"id":7,"result":{"targetId":"A"}}"#) else {
            panic!("not a reply");
        };
        assert_eq!(id, 7);
        assert_eq!(r.unwrap()["targetId"], "A");
    }

    #[test]
    fn reply_with_error() {
        let Some(Incoming::Reply(id, r)) = parse(br#"{"id":3,"error":{"code":-32000,"message":"No target"}}"#) else {
            panic!("not a reply");
        };
        assert_eq!(id, 3);
        assert_eq!(r.unwrap_err(), "No target");
    }

    #[test]
    fn events_carry_their_session() {
        let Some(Incoming::Event(e)) =
            parse(br#"{"method":"Page.frameNavigated","params":{"frame":{}},"sessionId":"S"}"#)
        else {
            panic!("not an event");
        };
        assert_eq!(e.method, "Page.frameNavigated");
        assert_eq!(e.session.as_deref(), Some("S"));
        assert!(parse(b"not json").is_none());
        assert!(parse(br#"{"params":{}}"#).is_none());
    }

    #[test]
    fn reader_splits_on_nul_and_routes_messages() {
        let pending: Pending = Arc::default();
        let (tx1, mut rx1) = oneshot::channel();
        let (tx2, mut rx2) = oneshot::channel();
        pending.lock().unwrap().insert(1, tx1);
        pending.lock().unwrap().insert(2, tx2);
        let (events, mut events_rx) = mpsc::unbounded_channel();
        let stream = b"{\"method\":\"X\"}\0{\"id\":2,\"result\":{}}\0{\"id\":1,\"error\":{\"message\":\"bad\"}}\0";
        read_loop(&stream[..], &pending, &events);
        assert_eq!(rx2.try_recv().unwrap().unwrap(), json!({}));
        assert_eq!(rx1.try_recv().unwrap().unwrap_err(), "bad");
        assert!(pending.lock().unwrap().is_empty());
        assert_eq!(events_rx.try_recv().unwrap().method, "X");
    }
}
