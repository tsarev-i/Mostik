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

impl CookieJar {
    /// Keeps only cookies of the Yandex domains.
    pub fn from_cookies(cookies: impl IntoIterator<Item = StoredCookie>) -> Self {
        let cookies = cookies
            .into_iter()
            .filter(|c| {
                let d = c.domain.trim_start_matches('.');
                d == "yandex.ru" || d.ends_with(".yandex.ru") || d == "yandex.net" || d.ends_with(".yandex.net")
            })
            .collect();
        Self { cookies }
    }

    /// Whether the jar holds a Yandex ID session.
    pub fn has_session(&self) -> bool {
        self.cookies
            .iter()
            .any(|c| c.name == SESSION_COOKIE && !c.value.is_empty())
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
}
