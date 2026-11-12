//! Service addresses as the `telemost.360.yandex.ru` web client uses them.

/// Web client on whose behalf requests are made: `Origin` and `Referer`.
pub const WEB_ORIGIN: &str = "https://telemost.360.yandex.ru";

/// HTTP RPC: `POST` with `multipart/form-data`, field `request`.
pub const API_URL: &str = "https://messenger.360.yandex.ru/api/";

/// Issues the CSRF token for the `X-CSRF-TOKEN` header.
pub const CSRF_URL: &str = "https://messenger.360.yandex.ru/csrf-token/";

/// Xiva realtime channel.
pub const XIVA_WS_URL: &str = "wss://ws.push.yandex.ru/v2/subscribe/websocket";

/// Xiva service the Messenger subscribes to.
pub const XIVA_SERVICE: &str = "messenger-prod";

/// Host of private files (uploads, previews, voice messages).
pub const FILES_HOST: &str = "files.messenger.yandex.ru";

/// Domains of Yandex: Telemost and Yandex ID live on `yandex.ru`, files and push on `yandex.net`,
/// and the international pages, sign-in included, on `yandex.com`.
const YANDEX_DOMAINS: [&str; 3] = ["yandex.ru", "yandex.com", "yandex.net"];

/// Whether `host` is one of [`YANDEX_DOMAINS`] or a subdomain of one: whose cookies the session
/// keeps, and which pages are Yandex's own in the call window.
pub fn is_yandex_host(host: &str) -> bool {
    YANDEX_DOMAINS.iter().any(|d| {
        host.strip_suffix(d)
            .is_some_and(|sub| sub.is_empty() || sub.ends_with('.'))
    })
}

/// Yandex ID login page that returns to the web client.
pub fn passport_login_url() -> String {
    format!("https://passport.yandex.ru/auth?retpath={WEB_ORIGIN}%2F")
}

/// Xiva WebSocket address. `session` is a random client UUID, `user` is the puid.
pub fn xiva_ws_url(session: &str, user: &str) -> String {
    format!("{XIVA_WS_URL}?service={XIVA_SERVICE}&session={session}&client=web&user={user}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_yandex_hosts() {
        for host in [
            "yandex.ru",
            "telemost.360.yandex.ru",
            "yandex.net",
            "passport.yandex.com",
        ] {
            assert!(is_yandex_host(host), "{host}");
        }
        for host in [
            "notyandex.ru",
            "yandex.ru.evil.example",
            "example.com",
            "yandex.org",
            "",
        ] {
            assert!(!is_yandex_host(host), "{host}");
        }
    }

    #[test]
    fn builds_xiva_url() {
        assert_eq!(
            xiva_ws_url("s-1", "42"),
            "wss://ws.push.yandex.ru/v2/subscribe/websocket?service=messenger-prod&session=s-1&client=web&user=42"
        );
    }
}
