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

/// Public feature configuration: reactions, intervals, flags.
pub const CLIENT_CONFIG_URL: &str = "https://tools.messenger.yandex.net/config.json";

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
    fn builds_xiva_url() {
        assert_eq!(
            xiva_ws_url("s-1", "42"),
            "wss://ws.push.yandex.ru/v2/subscribe/websocket?service=messenger-prod&session=s-1&client=web&user=42"
        );
    }
}
