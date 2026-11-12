//! WS request bodies in the web client format (protobuf JSON, PascalCase fields).
//!
//! Only what was confirmed by studying the web client is here.

use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

/// Fresh `RequestId` for a WS request.
pub fn request_id() -> String {
    Uuid::new_v4().to_string()
}

/// `history` request. Without `chat_id` it returns the chat list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct HistoryRequest {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_timestamp: Option<u64>,
    /// Always 0, as the web client sends it.
    pub offset: u32,
}

impl HistoryRequest {
    /// The last `limit` messages of a chat before `before` (exclusive).
    pub fn page(chat_id: &str, limit: u32, before: Option<u64>) -> Self {
        Self {
            request_id: request_id(),
            chat_id: Some(chat_id.to_owned()),
            limit,
            max_timestamp: before,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ClientTransportId {
    pub xiva_subscription_id: String,
}

/// `push` request: sends any `ClientMessage`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PushRequest {
    pub request_id: String,
    pub client_transport_id: ClientTransportId,
    pub user_agent: String,
    pub client_message: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl PushRequest {
    pub fn new(subscription_id: &str, user_agent: &str, client_message: Value) -> Self {
        Self {
            request_id: request_id(),
            client_transport_id: ClientTransportId {
                xiva_subscription_id: subscription_id.to_owned(),
            },
            user_agent: user_agent.to_owned(),
            client_message,
            meta: None,
        }
    }

    /// The web client takes `Meta.Origin` from the numeric `serviceId`.
    pub fn with_origin(mut self, service_id: Option<&str>) -> Self {
        if let Some(origin) = service_id.and_then(|s| s.parse::<u64>().ok()) {
            self.meta = Some(json!({ "Origin": origin }));
        }
        self
    }
}

/// Web client `ReactionAction`: `REMOVE = 1`, `REPLACE = 2`.
const REACTION_REMOVE: u32 = 1;
const REACTION_REPLACE: u32 = 2;

/// Puts (`add = true`) or removes a reaction of type `kind` (ID `1001xx`).
pub fn reaction(chat_id: &str, ts: u64, kind: u64, add: bool) -> Value {
    let action = if add { REACTION_REPLACE } else { REACTION_REMOVE };
    json!({ "Reaction": { "ChatId": chat_id, "Timestamp": ts, "Type": kind, "Action": action } })
}

/// "Important" flag: `ImportanceFlag` 1 sets it, 2 clears it.
pub fn importance(chat_id: &str, ts: u64, important: bool) -> Value {
    json!({ "UpdateFields": { "ChatId": chat_id, "Timestamp": ts, "ImportanceFlag": if important { 1 } else { 2 } } })
}

/// Marks the chat as read up to message `ts` with number `seq_no`.
pub fn seen_marker(chat_id: &str, ts: u64, seq_no: u64) -> Value {
    json!({ "SeenMarker": { "ChatId": chat_id, "Timestamp": ts, "SeqNo": seq_no } })
}

/// Chat list, as the web client asks for it: `requestHistory({Limit: 1, ChatDataFilter: {}, MinTimestamp})`.
pub fn chat_list(min_timestamp: u64) -> Value {
    json!({ "RequestId": request_id(), "Limit": 1, "ChatDataFilter": {}, "MinTimestamp": min_timestamp })
}

/// Threads, as the web client asks for them: `requestHistory({Threads: true, Limit: 0, ChatDataFilter: {}, MinTimestamp})`.
pub fn thread_list(min_timestamp: u64) -> Value {
    json!({ "RequestId": request_id(), "Threads": true, "Limit": 0, "ChatDataFilter": {}, "MinTimestamp": min_timestamp })
}

/// New text message. `payload_id` is a client UUID that deduplicates retries. The people
/// mentioned as `@<guid>` go into `MentionedUserIds`, as the web client sends them: Telemost
/// notifies them and returns their names in `MentionedUsers`.
pub fn plain_text(chat_id: &str, payload_id: &str, text: &str) -> Value {
    let mut plain = json!({
        "ChatId": chat_id,
        "PayloadId": payload_id,
        "Text": { "MessageText": text },
    });
    let mentioned = mostik_core::markup::mentioned_ids(text);
    if !mentioned.is_empty() {
        plain["MentionedUserIds"] = json!(mentioned);
    }
    json!({ "Plain": plain })
}

/// Reply to message `source_ts` of chat `source_chat`, as the web client sends it: the original as
/// the only forwarded message from the same chat (a thread chat for a thread reply). A quoted
/// fragment goes into `ForwardedMessageStyles[0].Quote`.
pub fn reply(
    chat_id: &str,
    payload_id: &str,
    text: &str,
    source_chat: &str,
    source_ts: u64,
    quote: Option<&str>,
) -> Value {
    let mut msg = plain_text(chat_id, payload_id, text);
    msg["Plain"]["ForwardedMessageRefs"] = json!([{ "ChatId": source_chat, "Timestamp": source_ts }]);
    if let Some(q) = quote.map(str::trim).filter(|q| !q.is_empty()) {
        msg["Plain"]["ForwardedMessageStyles"] = json!([{ "Quote": q }]);
    }
    msg
}

/// Edit: the same `Plain`, but with the `Timestamp` of the original message.
pub fn edit_text(chat_id: &str, ts: u64, payload_id: &str, text: &str) -> Value {
    let mut msg = plain_text(chat_id, payload_id, text);
    msg["Plain"]["Timestamp"] = json!(ts);
    msg
}

/// Edit of a message with more than text (a gallery, a file, forwarded messages): the whole
/// original `Plain` with the new text, as the web client sends it. A `Plain` with only `Text`
/// would turn such a message into plain text and drop its images and files.
pub fn edit_whole(original: &Value, chat_id: &str, ts: u64, text: &str) -> Value {
    let mut plain = original.clone();
    if !plain.is_object() {
        plain = json!({});
    }
    plain["ChatId"] = json!(chat_id);
    plain["Timestamp"] = json!(ts);
    if plain.get("Gallery").is_some() {
        plain["Gallery"]["Text"] = json!(text);
    } else {
        plain["Text"] = json!({ "MessageText": text });
    }
    let mentioned = mostik_core::markup::mentioned_ids(text);
    match plain.as_object_mut() {
        Some(o) if mentioned.is_empty() => {
            o.remove("MentionedUserIds");
        }
        Some(o) => {
            o.insert("MentionedUserIds".into(), json!(mentioned));
        }
        None => {}
    }
    json!({ "Plain": plain })
}

/// Delete: `Plain` with only `ChatId` and `Timestamp`.
pub fn delete(chat_id: &str, ts: u64) -> Value {
    json!({ "Plain": { "ChatId": chat_id, "Timestamp": ts } })
}

/// Typing indicator.
pub fn typing(chat_id: &str) -> Value {
    json!({ "Typing": { "ChatId": chat_id } })
}

/// Tells Telemost that a call rings on this device (`NotifyRinging`), as the web client does when
/// it shows an incoming call.
pub fn notify_ringing(ringing_id: &str) -> Value {
    json!({ "MeetingCallingMessage": { "RingingId": ringing_id, "NotifyRinging": {} } })
}

/// Why the user stopped a call ringing: Telemost's `EndRingingReason`.
#[derive(Debug, Clone, Copy)]
pub enum EndRinging {
    Accepted = 0,
    Declined = 1,
}

/// Stops a call ringing on all the user's devices: accepted or declined (`EndRinging`; the
/// supported feature 0 is `END_ACTION`).
pub fn end_ringing(ringing_id: &str, reason: EndRinging) -> Value {
    json!({ "MeetingCallingMessage": { "RingingId": ringing_id,
        "EndRinging": { "Reason": reason as u8, "SupportedFeatures": [0] } } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ringing_answers_carry_the_ringing_id() {
        assert_eq!(
            end_ringing("r1", EndRinging::Declined),
            json!({"MeetingCallingMessage": {"RingingId": "r1",
                "EndRinging": {"Reason": 1, "SupportedFeatures": [0]}}})
        );
        assert_eq!(
            end_ringing("r1", EndRinging::Accepted)["MeetingCallingMessage"]["EndRinging"]["Reason"],
            0
        );
        assert_eq!(
            notify_ringing("r1"),
            json!({"MeetingCallingMessage": {"RingingId": "r1", "NotifyRinging": {}}})
        );
    }

    #[test]
    fn history_page_omits_empty_fields() {
        let mut req = HistoryRequest::page("0/0/abc", 50, Some(1_700_000_000_000_000));
        req.request_id = "r".into();
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({"RequestId": "r", "ChatId": "0/0/abc", "Limit": 50, "MaxTimestamp": 1_700_000_000_000_000u64, "Offset": 0})
        );
    }

    #[test]
    fn plain_edit_delete_shapes() {
        let id = "aaaa0001-0000-4000-8000-000000000001";
        let text = format!("@{id} hello");
        let send = plain_text("c", "p", &text);
        assert_eq!(
            send,
            json!({"Plain": {"ChatId": "c", "PayloadId": "p", "Text": {"MessageText": text}, "MentionedUserIds": [id]}})
        );
        let edit = edit_text("c", 42, "p", "edited");
        assert_eq!(edit["Plain"]["Timestamp"], 42);
        assert!(edit["Plain"].get("MentionedUserIds").is_none());
        assert_eq!(edit_text("c", 42, "p", &text)["Plain"]["MentionedUserIds"], json!([id]));
        assert_eq!(delete("c", 42), json!({"Plain": {"ChatId": "c", "Timestamp": 42}}));
    }

    #[test]
    fn reply_and_quote_shapes() {
        assert_eq!(
            reply("c", "p", "sure", "c", 42, None),
            json!({"Plain": {"ChatId": "c", "PayloadId": "p", "Text": {"MessageText": "sure"},
                "ForwardedMessageRefs": [{"ChatId": "c", "Timestamp": 42}]}})
        );
        assert_eq!(
            reply("c", "p", "sure", "c", 42, Some("  ")),
            reply("c", "p", "sure", "c", 42, None)
        );
        let quote = reply("100/0/u_1", "p", "why?", "100/0/u_1", 42, Some(" this **part** "));
        assert_eq!(
            quote["Plain"]["ForwardedMessageStyles"],
            json!([{"Quote": "this **part**"}])
        );
        assert_eq!(quote["Plain"]["ForwardedMessageRefs"][0]["ChatId"], "100/0/u_1");
        // Editing a reply resends the whole message: the reference and the quote stay.
        let edit = edit_whole(&quote["Plain"], "100/0/u_1", 7, "edited");
        assert_eq!(
            edit["Plain"]["ForwardedMessageStyles"],
            quote["Plain"]["ForwardedMessageStyles"]
        );
        assert_eq!(
            edit["Plain"]["ForwardedMessageRefs"],
            quote["Plain"]["ForwardedMessageRefs"]
        );
    }

    #[test]
    fn whole_edit_keeps_the_gallery_and_the_files() {
        let items =
            json!([{"Image": {"FileInfo": {"Id2": "file/x", "Name": "a.png", "Size": 3}, "Width": 1, "Height": 1}}]);
        let gallery = json!({"ChatId": "c", "PayloadId": "p", "Gallery": {"Items": items, "Text": "old"},
            "MentionedUserIds": ["gone"]});
        let edit = edit_whole(&gallery, "c", 42, "new");
        assert_eq!(
            edit,
            json!({"Plain": {"ChatId": "c", "PayloadId": "p", "Timestamp": 42, "Gallery": {"Items": items, "Text": "new"}}})
        );
        let id = "aaaa0001-0000-4000-8000-000000000001";
        let forward = json!({"ChatId": "c", "ForwardedMessages": [{"x": 1}], "Text": {"MessageText": "old"}});
        let edit = edit_whole(&forward, "c", 7, &format!("@{id}"));
        assert_eq!(edit["Plain"]["ForwardedMessages"], json!([{"x": 1}]));
        assert_eq!(edit["Plain"]["Text"]["MessageText"], format!("@{id}"));
        assert_eq!(edit["Plain"]["MentionedUserIds"], json!([id]));
    }

    #[test]
    fn write_actions_shapes() {
        assert_eq!(reaction("c", 5, 100102, true)["Reaction"]["Action"], 2);
        assert_eq!(reaction("c", 5, 100102, false)["Reaction"]["Action"], 1);
        assert_eq!(importance("c", 5, false)["UpdateFields"]["ImportanceFlag"], 2);
        assert_eq!(
            seen_marker("c", 5, 9),
            json!({"SeenMarker": {"ChatId": "c", "Timestamp": 5, "SeqNo": 9}})
        );
        assert_eq!(chat_list(0)["ChatDataFilter"], json!({}));
        let push = PushRequest::new("s", "UA", typing("c")).with_origin(Some("123"));
        assert_eq!(push.meta, Some(json!({"Origin": 123})));
        assert_eq!(
            PushRequest::new("s", "UA", typing("c")).with_origin(Some("abc")).meta,
            None
        );
    }

    #[test]
    fn push_request_wraps_message() {
        let mut req = PushRequest::new("sub", "UA", typing("c"));
        req.request_id = "r".into();
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({"RequestId": "r", "ClientTransportId": {"XivaSubscriptionId": "sub"}, "UserAgent": "UA", "ClientMessage": {"Typing": {"ChatId": "c"}}})
        );
    }
}
