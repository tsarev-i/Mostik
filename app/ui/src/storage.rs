//! `localStorage`, where the UI keeps its preferences and what it remembers on this device. A
//! browser may give none (a private window) or refuse a write: then nothing is kept and the
//! defaults apply.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The page's `localStorage`, if there is one.
pub fn local() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The text kept under `key`.
pub fn get(key: &str) -> Option<String> {
    local()?.get_item(key).ok().flatten()
}

/// Keeps `value` under `key`.
pub fn set(key: &str, value: &str) {
    if let Some(s) = local() {
        let _ = s.set_item(key, value);
    }
}

/// Forgets `key`.
pub fn remove(key: &str) {
    if let Some(s) = local() {
        let _ = s.remove_item(key);
    }
}

/// The value kept under `key` as JSON; `None` when there is none or it does not parse.
pub fn load<T: DeserializeOwned>(key: &str) -> Option<T> {
    serde_json::from_str(&get(key)?).ok()
}

/// Keeps `value` under `key` as JSON.
pub fn save<T: Serialize + ?Sized>(key: &str, value: &T) {
    if let Ok(v) = serde_json::to_string(value) {
        set(key, &v);
    }
}
