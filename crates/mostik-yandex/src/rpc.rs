//! HTTP RPC `messenger.360.yandex.ru/api/`.
//!
//! The request body is `multipart/form-data` with a single `request` field
//! holding the JSON `{"method": "...", "params": {...}}`.

use serde::Serialize;
use serde_json::Value;

/// Form field in which the web client sends the request.
pub const FORM_FIELD: &str = "request";

/// API version the web client currently uses.
pub const API_VERSION: u32 = 5;

/// `{method, params}` envelope of an RPC call.
#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest<'a, P: Serialize> {
    pub method: &'a str,
    pub params: P,
}

impl<'a, P: Serialize> RpcRequest<'a, P> {
    pub fn new(method: &'a str, params: P) -> Self {
        Self { method, params }
    }

    /// Value of the `request` form field.
    pub fn form_value(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// The web client sends a request without parameters with `params: {}`.
pub fn empty_params() -> Value {
    Value::Object(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_like_web_client() {
        let req = RpcRequest::new("get_chat_members", json!({"chat_id": "0/0/x", "limit": 50}));
        let v: Value = serde_json::from_str(&req.form_value().unwrap()).unwrap();
        assert_eq!(
            v,
            json!({"method": "get_chat_members", "params": {"chat_id": "0/0/x", "limit": 50}})
        );
    }

    #[test]
    fn empty_params_is_object() {
        let req = RpcRequest::new("whoami", empty_params());
        assert_eq!(req.form_value().unwrap(), r#"{"method":"whoami","params":{}}"#);
    }
}
