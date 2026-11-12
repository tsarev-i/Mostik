//! Live write self-test on a real account.
//!
//! Works **only** in a group named `test` that has exactly one member (the user).
//! Runs: send → edit → reaction (add, remove) → "important" → typing →
//! read marker → delete → thread reply → reply and quote, checking every step against the chat history.
//!
//! Run: `cargo run -p mostik-app --features selftest --bin mostik-selftest` (needs a session saved by Mostik).
//! `-- --threads` only prints the format of thread chat IDs (read-only).
//! `-- --listen` prints every raw push as JSON until stopped (read-only; includes message texts).
//! `-- --dump [list | <chat id>]` prints the raw chat list or the latest page of a chat (read-only).
//! `-- --buckets` prints the synced settings, `get_buckets` (read-only).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mostik_app::store;
use mostik_yandex::diag::{id_pattern, shape};
use mostik_yandex::identity;
use mostik_yandex::web::WebClient;
use mostik_yandex::wire::{self, HistoryRequest, PushRequest, request_id};
use mostik_yandex::xiva_client::{Incoming, XivaClient};
use serde_json::{Value, json};

const TEST_CHAT_NAME: &str = "test";
const SETTLE: Duration = Duration::from_millis(800);

type Res<T> = Result<T, String>;

struct Ctx {
    xiva: XivaClient,
    sub: String,
    ua: String,
    origin: Option<String>,
    chat: String,
}

impl Ctx {
    async fn push(&self, msg: Value) -> Res<Value> {
        let req = PushRequest::new(&self.sub, &self.ua, msg).with_origin(self.origin.as_deref());
        self.xiva.call("push", &req, 20).await.map_err(|e| e.to_string())
    }

    /// The latest messages of the test chat (raw `ServerMessage`s).
    async fn recent(&self) -> Res<Vec<Value>> {
        let v = self
            .xiva
            .call("history", &HistoryRequest::page(&self.chat, 20, None), 20)
            .await
            .map_err(|e| e.to_string())?;
        Ok(v.pointer("/Chats/0/Messages")
            .and_then(Value::as_array)
            .map(|m| {
                m.iter()
                    .map(|x| x.get("ServerMessage").cloned().unwrap_or(x.clone()))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn find(&self, ts: u64) -> Res<Option<Value>> {
        Ok(self
            .recent()
            .await?
            .into_iter()
            .find(|m| m.pointer("/ServerMessageInfo/Timestamp").and_then(Value::as_u64) == Some(ts)))
    }
}

fn step(name: &str, ok: bool, detail: impl std::fmt::Display) -> bool {
    println!("{} {name}: {detail}", if ok { "✅" } else { "❌" });
    ok
}

fn sent_ts(resp: &Value) -> Res<u64> {
    resp.pointer("/MessageInfo/TimestampMcs")
        .and_then(Value::as_u64)
        .ok_or_else(|| "no MessageInfo.TimestampMcs".into())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run().await {
        println!("❌ stopped: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Res<()> {
    let jar = store::load().ok_or("no saved session: sign in with Mostik first")?;
    let ua = identity::user_agent(identity::BROWSER_UA);
    let mut web = WebClient::new(jar, ua.clone()).map_err(|e| e.to_string())?;
    let page = web.load_page().await.map_err(|e| e.to_string())?;
    if let Some(i) = std::env::args().position(|a| a == "--chat-info") {
        let ids: Vec<String> = std::env::args().skip(i + 1).collect();
        let v = web
            .rpc("get_chats_info", json!({ "chat_ids": ids }))
            .await
            .map_err(|e| e.to_string())?;
        println!("{v}");
        return Ok(());
    }
    if std::env::args().any(|a| a == "--buckets") {
        let v = web.buckets().await.map_err(|e| e.to_string())?;
        println!("{v}");
        return Ok(());
    }

    let pushes: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = pushes.clone();
    let listen = std::env::args().any(|a| a == "--listen");
    let xiva = XivaClient::connect(web.jar(), &ua, &page.puid, move |inc| {
        if listen {
            let now = mostik_yandex::clock::now_ms();
            match &inc {
                Incoming::Push { event, body } => println!("{}", json!({ "t": now, "event": event, "body": body })),
                Incoming::Text(t) => println!("{}", json!({ "t": now, "text": t })),
                Incoming::Closed => println!("{}", json!({ "t": now, "closed": true })),
            }
        }
        if let Incoming::Push { body, .. } = inc
            && let Ok(mut p) = sink.lock()
        {
            p.push(body);
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let sub = xiva
        .wait_subscription(Duration::from_secs(5))
        .await
        .ok_or("no Xiva subscription")?;

    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--dump") {
        let req: Value = match args.get(i + 1).map(String::as_str) {
            Some("list") | None => serde_json::to_value(wire::chat_list(0)).map_err(|e| e.to_string())?,
            Some(chat) => serde_json::to_value(HistoryRequest::page(chat, 30, None)).map_err(|e| e.to_string())?,
        };
        let v = xiva.call("history", &req, 40).await.map_err(|e| e.to_string())?;
        println!("{v}");
        return Ok(());
    }
    if listen {
        println!("listening, subscription {sub}");
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
    if std::env::args().any(|a| a == "--threads") {
        return threads(&xiva).await;
    }

    // Find the test group and make sure the user is its only member.
    let list = xiva
        .call("history", &wire::chat_list(0), 40)
        .await
        .map_err(|e| e.to_string())?;
    let candidates: Vec<&Value> = list
        .get("Chats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| {
            c.pointer("/ChatInfo/Name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.trim().eq_ignore_ascii_case(TEST_CHAT_NAME))
        })
        .collect();
    let [chat] = candidates.as_slice() else {
        return Err(format!(
            "exactly one group named {TEST_CHAT_NAME:?} is required, found {}",
            candidates.len()
        ));
    };
    let members = chat
        .pointer("/ChatInfo/MemberCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if members != 1 {
        return Err(format!(
            "group {TEST_CHAT_NAME:?} has {members} members, exactly one is required"
        ));
    }
    let chat_id = chat["ChatId"].as_str().ok_or("no ChatId")?.to_owned();
    println!("group {TEST_CHAT_NAME:?} found, members: {members}");
    let ctx = Ctx {
        xiva,
        sub,
        ua,
        origin: page.service_id.clone(),
        chat: chat_id.clone(),
    };
    let mut all_ok = true;

    // 1. Send.
    let text = format!("Mostik: self-test {}", request_id());
    let resp = ctx.push(wire::plain_text(&chat_id, &request_id(), &text)).await?;
    println!("   push response: {}", shape(&resp));
    let ts = sent_ts(&resp)?;
    tokio::time::sleep(SETTLE).await;
    let m = ctx.find(ts).await?;
    let got = m
        .as_ref()
        .and_then(|m| m.pointer("/ClientMessage/Plain/Text/MessageText"))
        .and_then(Value::as_str);
    all_ok &= step("send", got == Some(text.as_str()), format!("ts={ts}"));
    let seq = m
        .as_ref()
        .and_then(|m| m.pointer("/ServerMessageInfo/SeqNo"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    // 2. Edit.
    let edited = format!("{text} (edited)");
    ctx.push(wire::edit_text(&chat_id, ts, &request_id(), &edited)).await?;
    tokio::time::sleep(SETTLE).await;
    let m = ctx.find(ts).await?;
    let got = m
        .as_ref()
        .and_then(|m| m.pointer("/ClientMessage/Plain/Text/MessageText"))
        .and_then(Value::as_str);
    let version = m
        .as_ref()
        .and_then(|m| m.pointer("/ServerMessageInfo/Version"))
        .and_then(Value::as_u64);
    all_ok &= step("edit", got == Some(edited.as_str()), format!("Version={version:?}"));

    // 3. Reaction: add and remove.
    ctx.push(wire::reaction(&chat_id, ts, 100102, true)).await?;
    tokio::time::sleep(SETTLE).await;
    let m = ctx.find(ts).await?.unwrap_or_default();
    let has = |m: &Value| {
        m.get("Reactions").and_then(Value::as_array).is_some_and(|r| {
            r.iter()
                .any(|x| x["Type"] == 100102 && x["Count"].as_u64().unwrap_or(0) > 0)
        })
    };
    all_ok &= step("reaction 👍", has(&m), json!(m.get("Reactions")));
    ctx.push(wire::reaction(&chat_id, ts, 100102, false)).await?;
    tokio::time::sleep(SETTLE).await;
    let m = ctx.find(ts).await?.unwrap_or_default();
    all_ok &= step("reaction removed", !has(&m), json!(m.get("Reactions")));

    // 4. "Important".
    let r = ctx.push(wire::importance(&chat_id, ts, true)).await?;
    tokio::time::sleep(SETTLE).await;
    let m = ctx.find(ts).await?.unwrap_or_default();
    step(
        "important (structure only)",
        true,
        format!("response {}, message {}", shape(&r), shape(&m)),
    );
    ctx.push(wire::importance(&chat_id, ts, false)).await?;

    // 5. Typing and read marker.
    let r = ctx.push(wire::typing(&chat_id)).await;
    all_ok &= step("typing", r.is_ok(), r.map(|v| shape(&v)).unwrap_or_else(|e| json!(e)));
    let r = ctx.push(wire::seen_marker(&chat_id, ts, seq)).await;
    all_ok &= step(
        "read marker",
        r.is_ok(),
        r.map(|v| shape(&v)).unwrap_or_else(|e| json!(e)),
    );

    // 6. Delete.
    ctx.push(wire::delete(&chat_id, ts)).await?;
    tokio::time::sleep(SETTLE).await;
    let m = ctx.find(ts).await?;
    let gone = m
        .as_ref()
        .is_none_or(|m| m.pointer("/ClientMessage/Plain/Text").is_none());
    all_ok &= step("delete", gone, m.map(|m| shape(&m)).unwrap_or(json!("not in history")));

    // 7. Thread: message → thread reply → check → delete both.
    let parent_text = format!("Mostik: thread parent {}", request_id());
    let r = ctx
        .push(wire::plain_text(&chat_id, &request_id(), &parent_text))
        .await?;
    let pts = sent_ts(&r)?;
    let thread = mostik_yandex::convert::thread_chat_id(&chat_id, pts).ok_or("this chat does not support threads")?;
    let r = ctx
        .push(wire::plain_text(&thread, &request_id(), "Mostik: thread reply"))
        .await;
    all_ok &= step(
        "thread reply",
        r.as_ref().is_ok_and(|v| v.get("MessageInfo").is_some()),
        r.as_ref().map(shape).unwrap_or_else(|e| json!(e)),
    );
    let rts = r
        .ok()
        .and_then(|v| v.pointer("/MessageInfo/TimestampMcs").and_then(Value::as_u64));
    tokio::time::sleep(Duration::from_secs(1)).await;
    let th = ctx
        .xiva
        .call("history", &HistoryRequest::page(&thread, 20, None), 20)
        .await
        .map_err(|e| e.to_string())?;
    let replies = th
        .pointer("/Chats/0/Messages")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    all_ok &= step("thread history", replies >= 1, format!("{replies} messages"));
    let parent = ctx.find(pts).await?.unwrap_or_default();
    let state = parent.pointer("/ServerMessageInfo/ThreadState");
    step("parent ThreadState", state.is_some(), json!(state.map(shape)));
    if let Some(rts) = rts {
        ctx.push(wire::delete(&thread, rts)).await?;
    }
    ctx.push(wire::delete(&chat_id, pts)).await?;

    // 8. Reply and quote: a message → a reply quoting part of it → Mostik reads both back → delete all.
    let original = format!("Mostik: reply target {}", request_id());
    let ots = sent_ts(&ctx.push(wire::plain_text(&chat_id, &request_id(), &original)).await?)?;
    for (name, q) in [("reply", None), ("quote", Some("reply target"))] {
        let r = ctx
            .push(wire::reply(
                &chat_id,
                &request_id(),
                &format!("Mostik: {name}"),
                &chat_id,
                ots,
                q,
            ))
            .await?;
        let rts = sent_ts(&r)?;
        tokio::time::sleep(SETTLE).await;
        let m = ctx.find(rts).await?.unwrap_or_default();
        let parsed =
            mostik_yandex::convert::from_history(&json!({"Chats": [{"ChatId": chat_id, "Messages": [m]}]}), None);
        let to = parsed.messages.first().and_then(|m| m.reply_to()).cloned();
        all_ok &= step(
            name,
            to.as_ref()
                .is_some_and(|f| f.ts == ots && f.text == original && f.quote.as_deref() == q),
            format!("message {}", shape(&m)),
        );
        ctx.push(wire::delete(&chat_id, rts)).await?;
    }
    ctx.push(wire::delete(&chat_id, ots)).await?;

    tokio::time::sleep(Duration::from_secs(1)).await;
    let pushes = pushes.lock().map(|p| p.clone()).unwrap_or_default();
    println!("pushes during the test: {}", pushes.len());
    for p in pushes.iter().take(12) {
        println!("   push: {}", shape(p));
    }
    if all_ok {
        Ok(())
    } else {
        Err("not all steps passed".into())
    }
}

/// Read-only check that a thread chat ID is `100/<ns>/<parent uuid>_<parent ts>`.
async fn threads(xiva: &XivaClient) -> Res<()> {
    let t = xiva
        .call("history", &wire::thread_list(0), 40)
        .await
        .map_err(|e| e.to_string())?;
    let chats = t.get("Chats").and_then(Value::as_array).cloned().unwrap_or_default();
    let patterns: BTreeSet<String> = chats
        .iter()
        .filter_map(|c| c["ChatId"].as_str())
        .map(id_pattern)
        .collect();
    println!("threads: {}, patterns: {patterns:?}", chats.len());
    for c in &chats {
        let id = c["ChatId"].as_str().unwrap_or_default();
        let parent_ts = c
            .pointer("/ThreadParentMessage/ServerMessageInfo/Timestamp")
            .and_then(Value::as_u64);
        let parent_chat = c
            .pointer("/ThreadParentMessage/ClientMessage/Plain/ChatId")
            .and_then(Value::as_str);
        let tail_ok = parent_ts.is_some_and(|ts| id.ends_with(&format!("_{ts}")));
        let uuid_ok = parent_chat
            .and_then(|p| p.rsplit('/').next())
            .is_some_and(|u| id.contains(u));
        step(
            "thread ID",
            tail_ok && uuid_ok,
            format!("suffix is parent time: {tail_ok}; contains parent chat uuid: {uuid_ok}"),
        );
    }
    Ok(())
}
