//! Signing out of Yandex ID the way the Telemost web client does.
//!
//! Its "Sign out" sends the browser to the page config's `passportLogoutUrl`,
//! `https://{passportHost}/passport?mode=logout&retpath={retpath}&yu={yandexuid}`, with `retpath`
//! its `logoutUrl` (`https://telemost.yandex.ru`) and `yu` the `yandexuid` cookie, which tells
//! Yandex ID the request comes from the session's own browser. Yandex ID ends the session and
//! leaves `Session_id` as `noauth:<time>`.

use std::time::Duration;

use reqwest::Url;
use reqwest::header::{COOKIE, LOCATION, REFERER, SET_COOKIE, USER_AGENT};

use crate::session::CookieJar;
use crate::web::WebError;

/// Yandex ID sign-out page.
const LOGOUT_PAGE: &str = "https://passport.yandex.ru/passport";

/// Where the web client asks Yandex ID to return after the sign-out (its `logoutUrl`).
const RETURN_TO: &str = "https://telemost.yandex.ru";

/// The cookie `yu` repeats.
const YANDEXUID: &str = "yandexuid";

/// Redirects followed within Yandex ID on the way out, before giving up.
const HOPS: usize = 5;

/// The sign-out address for a session, `None` without the `yandexuid` cookie.
pub fn logout_url(jar: &CookieJar) -> Option<Url> {
    let uid = jar
        .cookies
        .iter()
        .find(|c| c.name == YANDEXUID && !c.value.is_empty())?;
    Url::parse_with_params(
        LOGOUT_PAGE,
        [("mode", "logout"), ("retpath", RETURN_TO), ("yu", uid.value.as_str())],
    )
    .ok()
}

/// Hosts of Yandex ID that the sign-out may pass through, never the page it returns to.
fn is_passport(url: &Url) -> bool {
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|h| h == "passport.yandex.ru" || h.ends_with(".passport.yandex.ru"))
}

/// Ends the Yandex ID session of `jar` on the server, so that the cookies stop working wherever
/// a copy of them is left. Succeeds once Yandex ID has replaced the session cookie; the page it
/// returns to is not loaded.
pub async fn logout(jar: &CookieJar, user_agent: &str) -> Result<(), WebError> {
    let mut url = logout_url(jar).ok_or(WebError::MissingParam("yandexuid cookie"))?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()?;
    let mut jar = jar.clone();
    for _ in 0..=HOPS {
        let host = url.host_str().unwrap_or_default().to_owned();
        let resp = http
            .get(url.clone())
            .header(COOKIE, jar.header_for(&host))
            .header(USER_AGENT, user_agent)
            .header(REFERER, "https://telemost.360.yandex.ru/")
            .send()
            .await?;
        for header in resp.headers().get_all(SET_COOKIE) {
            if let Ok(header) = header.to_str() {
                jar.set_cookie(&host, header);
            }
        }
        if !jar.has_session() {
            return Ok(());
        }
        let status = resp.status();
        let next = resp
            .headers()
            .get(LOCATION)
            .and_then(|l| l.to_str().ok())
            .and_then(|l| url.join(l).ok())
            .filter(|next| status.is_redirection() && is_passport(next));
        match next {
            Some(next) => url = next,
            None => {
                return Err(WebError::BadResponse(format!(
                    "Yandex ID did not end the session (HTTP {status} from {host})"
                )));
            }
        }
    }
    Err(WebError::BadResponse("Yandex ID sign-out: too many redirects".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::StoredCookie;

    fn jar(cookies: &[(&str, &str)]) -> CookieJar {
        CookieJar {
            cookies: cookies
                .iter()
                .map(|(name, value)| StoredCookie {
                    name: (*name).into(),
                    value: (*value).into(),
                    domain: "yandex.ru".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn logout_url_is_the_web_clients() {
        let url = logout_url(&jar(&[("Session_id", "3:abc"), ("yandexuid", "1234567891700000000")])).unwrap();
        assert_eq!(
            url.as_str(),
            "https://passport.yandex.ru/passport?mode=logout&retpath=https%3A%2F%2Ftelemost.yandex.ru&yu=1234567891700000000"
        );
        assert!(logout_url(&jar(&[("Session_id", "3:abc")])).is_none());
    }

    #[test]
    fn the_sign_out_stays_within_yandex_id() {
        let ok = |u: &str| is_passport(&Url::parse(u).unwrap());
        assert!(ok("https://passport.yandex.ru/passport?mode=logout"));
        assert!(ok("https://sso.passport.yandex.ru/push?uuid=1"));
        assert!(!ok("https://telemost.yandex.ru/"));
        assert!(!ok("http://passport.yandex.ru/"));
        assert!(!ok("https://passport.yandex.ru.example.org/"));
    }

    #[test]
    fn a_noauth_session_cookie_is_the_end_of_the_session() {
        let mut j = jar(&[("Session_id", "3:abc"), ("yandexuid", "1")]);
        assert!(j.has_session());
        j.set_cookie(
            "passport.yandex.ru",
            "Session_id=noauth:1700000000; Domain=.yandex.ru; Path=/",
        );
        assert!(!j.has_session());
    }
}
