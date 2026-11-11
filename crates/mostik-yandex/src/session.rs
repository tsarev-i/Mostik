//! Yandex ID session cookies taken from the login window.

use serde::{Deserialize, Serialize};

/// A single cookie as it is stored between runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCookie {
    pub name: String,
    pub value: String,
    /// Cookie domain, for example `.yandex.ru`.
    pub domain: String,
}

/// Session cookies of the Yandex domains.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieJar {
    pub cookies: Vec<StoredCookie>,
}

/// Without this cookie there is no Yandex ID session.
pub const SESSION_COOKIE: &str = "Session_id";

/// Domains whose cookies the jar keeps.
fn is_yandex(domain: &str) -> bool {
    let d = domain.trim_start_matches('.');
    d == "yandex.ru" || d.ends_with(".yandex.ru") || d == "yandex.net" || d.ends_with(".yandex.net")
}

impl CookieJar {
    /// Keeps only cookies of the Yandex domains.
    pub fn from_cookies(cookies: impl IntoIterator<Item = StoredCookie>) -> Self {
        let cookies = cookies.into_iter().filter(|c| is_yandex(&c.domain)).collect();
        Self { cookies }
    }

    /// Applies a `Set-Cookie` header that `host` sent, the way a browser does: Yandex hands out
    /// some cookies on redirects (`yandex360session`, which lives five days, comes from
    /// `cookier.360.yandex.ru`), and without them the Telemost page keeps redirecting.
    /// Cookies of other domains, or of a domain `host` may not set, are ignored; an expired
    /// cookie is removed.
    pub fn set_cookie(&mut self, host: &str, header: &str) {
        let Ok(parsed) = cookie::Cookie::parse(header) else {
            return;
        };
        let domain = parsed
            .domain()
            .map(|d| d.trim_start_matches('.').to_ascii_lowercase())
            .unwrap_or_else(|| host.to_owned());
        if !(host == domain || host.ends_with(&format!(".{domain}"))) || !is_yandex(&domain) {
            return;
        }
        let expired = parsed.max_age().is_some_and(|age| age <= cookie::time::Duration::ZERO)
            || parsed
                .expires_datetime()
                .is_some_and(|at| at <= cookie::time::OffsetDateTime::now_utc());
        self.cookies
            .retain(|c| !(c.name == parsed.name() && c.domain.trim_start_matches('.') == domain));
        if !expired {
            self.cookies.push(StoredCookie {
                name: parsed.name().to_owned(),
                value: parsed.value().to_owned(),
                domain,
            });
        }
    }

    /// Whether the jar holds a Yandex ID session. After a sign-out Yandex leaves `Session_id`
    /// in place with a `noauth:<time>` value, which is no session.
    pub fn has_session(&self) -> bool {
        self.cookies
            .iter()
            .any(|c| c.name == SESSION_COOKIE && !c.value.is_empty() && !c.value.starts_with("noauth"))
    }

    /// `Cookie` header value for a request to `host`.
    pub fn header_for(&self, host: &str) -> String {
        self.cookies
            .iter()
            .filter(|c| {
                let d = c.domain.trim_start_matches('.');
                host == d || host.ends_with(&format!(".{d}"))
            })
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, domain: &str) -> StoredCookie {
        StoredCookie {
            name: name.into(),
            value: "v".into(),
            domain: domain.into(),
        }
    }

    #[test]
    fn filters_foreign_domains_and_matches_hosts() {
        let jar = CookieJar::from_cookies([
            c("Session_id", ".yandex.ru"),
            c("tm", "telemost.360.yandex.ru"),
            c("evil", ".example.com"),
            c("fake", ".notyandex.ru"),
        ]);
        assert_eq!(jar.cookies.len(), 2);
        assert!(jar.has_session());
        assert_eq!(jar.header_for("messenger.360.yandex.ru"), "Session_id=v");
        assert_eq!(jar.header_for("telemost.360.yandex.ru"), "Session_id=v; tm=v");
        assert_eq!(jar.header_for("yandex.com"), "");
    }

    #[test]
    fn set_cookie_adds_replaces_and_removes() {
        let mut jar = CookieJar::from_cookies([c("Session_id", ".yandex.ru"), c("yandex360session", ".yandex.ru")]);
        let host = "cookier.360.yandex.ru";
        jar.set_cookie(host, "yandex360session=new; Domain=.yandex.ru; Path=/; Secure");
        assert_eq!(
            jar.header_for("telemost.360.yandex.ru"),
            "Session_id=v; yandex360session=new"
        );
        jar.set_cookie(host, "tmp=1; Path=/");
        assert_eq!(jar.header_for(host), "Session_id=v; yandex360session=new; tmp=1");
        assert_eq!(
            jar.header_for("telemost.360.yandex.ru"),
            "Session_id=v; yandex360session=new"
        );
        jar.set_cookie(host, "tmp=; Max-Age=0");
        jar.set_cookie(
            host,
            "yandex360session=; Expires=Thu, 01 Jan 1970 00:00:01 GMT; Domain=.yandex.ru",
        );
        assert_eq!(jar.header_for(host), "Session_id=v");
    }

    #[test]
    fn set_cookie_ignores_foreign_domains() {
        let mut jar = CookieJar::default();
        jar.set_cookie("telemost.360.yandex.ru", "a=1; Domain=.example.com");
        jar.set_cookie("telemost.360.yandex.ru", "b=1; Domain=passport.yandex.ru");
        jar.set_cookie("example.com", "c=1");
        assert!(jar.cookies.is_empty());
    }

    #[test]
    fn signed_out_cookie_is_no_session() {
        let mut cookie = c("Session_id", ".yandex.ru");
        cookie.value = "noauth:1762850000".into();
        assert!(!CookieJar::from_cookies([cookie]).has_session());
    }
}
