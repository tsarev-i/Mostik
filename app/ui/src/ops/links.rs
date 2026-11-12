//! Links: message links, and opening a link from a message in Mostik or in the browser.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::Ctx;
use crate::bridge;

use super::chats::jump_to_message;
use super::report;

/// Message link, like "Copy link" in Slack: the web client route `/chats/<chat>/<time>`.
pub fn message_link(m: &Message) -> String {
    let ts = m.thread_parent.unwrap_or(m.ts);
    format!(
        "{LINK_BASE}/chats/{}/{ts}",
        String::from(js_sys::encode_uri_component(&m.chat_id.0))
    )
}

const LINK_BASE: &str = "https://messenger.360.yandex.ru";

/// Telemost message link → `(chat, time)`, so that it opens in Mostik rather than in the browser.
fn parse_message_link(url: &str) -> Option<(ChatId, Timestamp)> {
    let (chat, ts) = split_message_link(url)?;
    let chat = String::from(js_sys::decode_uri_component(chat).ok()?);
    Some((ChatId(chat), ts))
}

/// `(encoded chat, time)` of a message link. Accepts `/chats/<chat>/<time>` with an optional
/// `/chat` prefix and an optional `#` route, as the web clients write it.
pub(super) fn split_message_link(url: &str) -> Option<(&str, Timestamp)> {
    let rest = ["https://messenger.360.yandex.ru", "https://telemost.360.yandex.ru"]
        .iter()
        .find_map(|base| url.strip_prefix(base))?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    // Only a whole `chat` segment is a prefix: `chats/…` must stay intact.
    let rest = rest
        .strip_prefix("chat/")
        .or_else(|| rest.strip_prefix("chat#"))
        .unwrap_or(rest);
    let rest = rest.trim_start_matches('#').trim_start_matches('/');
    let rest = rest.strip_prefix("chats/")?;
    let (chat, ts) = rest.split_once('/')?;
    (!chat.is_empty()).then_some(())?;
    Some((chat, ts.trim_end_matches('/').parse().ok()?))
}

/// A link from a message: in the app it opens in the default browser, in the demo in a new tab.
pub fn open_url(c: Ctx, url: String) {
    if let Some((chat, ts)) = parse_message_link(&url)
        && c.snap.with_untracked(|s| s.chat(&chat).is_some())
    {
        return jump_to_message(c, chat, ts, None);
    }
    if !c.live {
        let _ = window().open_with_url_and_target(&url, "_blank");
        return;
    }
    spawn_local(async move {
        if let Err(e) = bridge::open_url(&url).await {
            report(c, e);
        }
    });
}
