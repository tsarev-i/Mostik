//! Binary frames of the Xiva WebSocket that the web client uses to talk to the Messenger.
//!
//! ```text
//! u8       kind: 1 is an RPC request or response, 3 is a server push
//! msgpack  header: kind 1 is [0, seq, "<method>"],
//!                  kind 3 is [<uid>, "messenger-prod", "delivery", <transit_id>]
//! u32 LE   protocol version (5)
//! u32 LE   auxiliary field: 0 in requests, non-zero in responses
//! u32 LE   0
//! JSON     body in protobuf JSON (PascalCase fields)
//! ```

use rmpv::Value as Mp;
use serde::Serialize;
use serde_json::Value;
use std::io::Cursor;

pub const FRAME_RPC: u8 = 1;
pub const FRAME_PUSH: u8 = 3;
pub const PROTOCOL_VERSION: u32 = 5;

/// The three words between the msgpack header and the JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub version: u32,
    pub aux: u32,
    pub reserved: u32,
}

impl Meta {
    pub const REQUEST: Meta = Meta {
        version: PROTOCOL_VERSION,
        aux: 0,
        reserved: 0,
    };
}

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// Client request or server response; the response carries the same `seq`.
    Rpc {
        seq: u64,
        method: String,
        meta: Meta,
        body: Value,
    },
    /// Server event (new message, typing, presence).
    Push {
        uid: String,
        service: String,
        event: String,
        transit_id: String,
        meta: Meta,
        body: Value,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("empty frame")]
    Empty,
    #[error("unknown frame kind {0}")]
    UnknownKind(u8),
    #[error("bad msgpack header: {0}")]
    Header(String),
    #[error("truncated frame")]
    Truncated,
    #[error("bad JSON body: {0}")]
    Json(#[from] serde_json::Error),
}

/// Encodes a client request.
pub fn encode_rpc(seq: u64, method: &str, body: &impl Serialize) -> Result<Vec<u8>, FrameError> {
    let mut out = vec![FRAME_RPC];
    let header = Mp::Array(vec![Mp::from(0u64), Mp::from(seq), Mp::from(method)]);
    rmpv::encode::write_value(&mut out, &header).map_err(|e| FrameError::Header(e.to_string()))?;
    write_meta(&mut out, Meta::REQUEST);
    serde_json::to_writer(&mut out, body)?;
    Ok(out)
}

fn write_meta(out: &mut Vec<u8>, meta: Meta) {
    for word in [meta.version, meta.aux, meta.reserved] {
        out.extend_from_slice(&word.to_le_bytes());
    }
}

/// The `seq` of a response frame, read from its header alone: a response whose body does not
/// decode can still be matched to its request.
pub fn rpc_seq(bytes: &[u8]) -> Option<u64> {
    let (&kind, rest) = bytes.split_first()?;
    if kind != FRAME_RPC {
        return None;
    }
    let header = rmpv::decode::read_value(&mut Cursor::new(rest)).ok()?;
    header.as_array()?.get(1)?.as_u64()
}

/// Decodes a frame received from the socket.
pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
    let (&kind, rest) = bytes.split_first().ok_or(FrameError::Empty)?;
    let mut cursor = Cursor::new(rest);
    let header = rmpv::decode::read_value(&mut cursor).map_err(|e| FrameError::Header(e.to_string()))?;
    let tail = &rest[cursor.position() as usize..];
    if tail.len() < 12 {
        return Err(FrameError::Truncated);
    }
    let word = |i: usize| u32::from_le_bytes([tail[i], tail[i + 1], tail[i + 2], tail[i + 3]]);
    let meta = Meta {
        version: word(0),
        aux: word(4),
        reserved: word(8),
    };
    let json = &tail[12..];
    let body = if json.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(json)?
    };
    let items = header
        .as_array()
        .ok_or_else(|| FrameError::Header("header is not an array".into()))?;
    let text = |i: usize| -> Result<String, FrameError> {
        items
            .get(i)
            .and_then(Mp::as_str)
            .map(str::to_owned)
            .ok_or_else(|| FrameError::Header(format!("no string at position {i}")))
    };
    match kind {
        FRAME_RPC => Ok(Frame::Rpc {
            seq: items
                .get(1)
                .and_then(Mp::as_u64)
                .ok_or_else(|| FrameError::Header("no seq".into()))?,
            method: text(2)?,
            meta,
            body,
        }),
        FRAME_PUSH => Ok(Frame::Push {
            uid: text(0)?,
            service: text(1)?,
            event: text(2)?,
            transit_id: text(3)?,
            meta,
            body,
        }),
        other => Err(FrameError::UnknownKind(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_response_that_does_not_decode_keeps_its_seq() {
        let mut frame = encode_rpc(42, "history", &json!({})).unwrap();
        frame.truncate(frame.len() - 1);
        frame.extend_from_slice(b"not json");
        assert!(decode(&frame).is_err());
        assert_eq!(rpc_seq(&frame), Some(42));
        assert_eq!(rpc_seq(&[]), None);
    }

    /// Start of a real `subscribe` frame from the web client: kind, header `[0, 19, "subscribe"]`,
    /// three words `5, 0, 0` and `{`.
    const OBSERVED_PREFIX: &[u8] = &[
        0x01, 0x93, 0x00, 0x13, 0xa9, b's', b'u', b'b', b's', b'c', b'r', b'i', b'b', b'e', 0x05, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, b'{',
    ];

    #[test]
    fn encodes_exactly_like_web_client() {
        let frame = encode_rpc(19, "subscribe", &json!({"RequestId": "r"})).unwrap();
        assert_eq!(&frame[..OBSERVED_PREFIX.len()], OBSERVED_PREFIX);
    }

    #[test]
    fn rpc_roundtrip() {
        let body = json!({"RequestId": "r1", "ChatId": "0/0/abc", "Limit": 50});
        let bytes = encode_rpc(300, "history", &body).unwrap();
        match decode(&bytes).unwrap() {
            Frame::Rpc {
                seq,
                method,
                meta,
                body: got,
            } => {
                assert_eq!(seq, 300);
                assert_eq!(method, "history");
                assert_eq!(meta, Meta::REQUEST);
                assert_eq!(got, body);
            }
            other => panic!("expected RPC, got {other:?}"),
        }
    }

    #[test]
    fn decodes_push() {
        let mut bytes = vec![FRAME_PUSH];
        let header = Mp::Array(vec![
            Mp::from("1130000000000001"),
            Mp::from("messenger-prod"),
            Mp::from("delivery"),
            Mp::from("transit-0001"),
        ]);
        rmpv::encode::write_value(&mut bytes, &header).unwrap();
        write_meta(
            &mut bytes,
            Meta {
                version: 5,
                aux: 0x8686_c84a,
                reserved: 0,
            },
        );
        bytes.extend_from_slice(br#"{"ClientMessage":{"Typing":{"ChatId":"0/0/abc"}}}"#);
        match decode(&bytes).unwrap() {
            Frame::Push {
                service,
                event,
                meta,
                body,
                ..
            } => {
                assert_eq!(service, "messenger-prod");
                assert_eq!(event, "delivery");
                assert_eq!(meta.aux, 0x8686_c84a);
                assert_eq!(body["ClientMessage"]["Typing"]["ChatId"], "0/0/abc");
            }
            other => panic!("expected a push, got {other:?}"),
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(decode(&[]), Err(FrameError::Empty)));
        assert!(matches!(
            decode(&[7, 0x90, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(FrameError::UnknownKind(7))
        ));
        assert!(matches!(
            decode(&[1, 0x93, 0, 1, 0xa1, b'x', 5, 0]),
            Err(FrameError::Truncated)
        ));
    }
}
