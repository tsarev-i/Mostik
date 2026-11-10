//! How Mostik introduces itself to Yandex servers.
//!
//! The client is unofficial and does not hide it: a suffix with the name, version and
//! the `unofficial` label is always appended to the `User-Agent`.

pub const CLIENT_NAME: &str = "Mostik";
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Suffix by which Yandex can tell Mostik traffic apart.
pub fn ua_suffix() -> String {
    format!("{CLIENT_NAME}/{CLIENT_VERSION} (unofficial Telemost client)")
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
    fn always_marks_client_as_unofficial() {
        let ua = user_agent("Mozilla/5.0 (X11; Linux x86_64)");
        assert!(ua.starts_with("Mozilla/5.0 (X11; Linux x86_64) Mostik/"));
        assert!(ua.ends_with("(unofficial Telemost client)"));
        assert_eq!(user_agent("  "), ua_suffix());
    }
}
