//! How Mostik introduces itself to Yandex servers.
//!
//! The client does not hide itself: its name and version are always appended to the `User-Agent`
//! of the embedded webview, the way desktop clients like Slack do.

pub const CLIENT_NAME: &str = "Mostik";
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Suffix by which Yandex can tell Mostik traffic apart: `Mostik/<version>`.
pub fn ua_suffix() -> String {
    format!("{CLIENT_NAME}/{CLIENT_VERSION}")
}

/// `User-Agent` for requests: the embedded webview UA plus the Mostik suffix.
pub fn user_agent(webview_ua: &str) -> String {
    let base = webview_ua.trim();
    if base.is_empty() {
        ua_suffix()
    } else {
        format!("{base} {}", ua_suffix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_names_mostik() {
        let ua = user_agent("Mozilla/5.0 (X11; Linux x86_64)");
        assert_eq!(ua, format!("Mozilla/5.0 (X11; Linux x86_64) Mostik/{CLIENT_VERSION}"));
        assert_eq!(user_agent("  "), format!("Mostik/{CLIENT_VERSION}"));
    }
}
