//! Session cookies are kept in the system password store via `keyring`: the Secret Service
//! (KWallet / GNOME Keyring) on Linux, the Credential Manager on Windows, the login keychain on macOS.
//!
//! The Credential Manager holds at most 1280 UTF-16 characters per credential, less than a typical
//! cookie jar, so there the secret is split into parts: `yandex-session`, `yandex-session.1`, …
//! On Linux and macOS it always fits into the first one.

use mostik_yandex::session::CookieJar;

const SERVICE: &str = "dev.mostik.client";
const ACCOUNT: &str = "yandex-session";

/// Lets a debug build keep its session under another service name, so that a test copy signs
/// in with its own account and never reads, rewrites or signs out the session of the copy the
/// user runs. Release builds ignore it.
const SERVICE_ENV: &str = "MOSTIK_KEYRING_SERVICE";

/// Longest part in UTF-16 code units: the Credential Manager keeps a password as UTF-16
/// in a blob of at most 2560 bytes.
#[cfg(windows)]
const PART_LEN: usize = 1280;
#[cfg(not(windows))]
const PART_LEN: usize = usize::MAX;

/// The service name the session is kept under: [`SERVICE`], or [`SERVICE_ENV`] in a debug build.
fn service() -> String {
    std::env::var(SERVICE_ENV)
        .ok()
        .filter(|s| cfg!(debug_assertions) && !s.is_empty())
        .unwrap_or_else(|| SERVICE.to_owned())
}

/// Saved session, if there is one and it still has the session cookie.
pub fn load() -> Option<CookieJar> {
    load_from(&service())
}

/// Saves the session in the password store.
pub fn save(jar: &CookieJar) -> Result<(), String> {
    save_to(&service(), jar)
}

/// Forgets the saved session. An error means it is still there: a locked or unreachable
/// password store would hand it to the next start.
pub fn delete() -> Result<(), String> {
    delete_parts(&service(), 0)
}

fn entry(service: &str, part: usize) -> keyring::Result<keyring::Entry> {
    if part == 0 {
        keyring::Entry::new(service, ACCOUNT)
    } else {
        keyring::Entry::new(service, &format!("{ACCOUNT}.{part}"))
    }
}

fn load_from(service: &str) -> Option<CookieJar> {
    let mut secret = String::new();
    for part in 0.. {
        match entry(service, part).and_then(|e| e.get_password()) {
            Ok(s) => secret.push_str(&s),
            Err(_) if part > 0 => break,
            Err(_) => return None,
        }
    }
    serde_json::from_str::<CookieJar>(&secret)
        .ok()
        .filter(CookieJar::has_session)
}

fn save_to(service: &str, jar: &CookieJar) -> Result<(), String> {
    let secret = serde_json::to_string(jar).map_err(|e| e.to_string())?;
    let parts = split(&secret, PART_LEN);
    for (i, part) in parts.iter().enumerate() {
        entry(service, i)
            .and_then(|e| e.set_password(part))
            .map_err(|e| format!("could not save the session to the password store: {e}"))?;
    }
    // Parts left over from a longer session would corrupt the next load.
    delete_parts(service, parts.len())
}

/// Deletes the parts from `first` on, up to the first missing one.
fn delete_parts(service: &str, first: usize) -> Result<(), String> {
    for part in first.. {
        match entry(service, part).and_then(|e| e.delete_credential()) {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => break,
            Err(e) => return Err(format!("could not delete the session from the password store: {e}")),
        }
    }
    Ok(())
}

/// Splits `s` into pieces of at most `max` UTF-16 code units, never inside a character.
fn split(s: &str, max: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut units) = (0, 0);
    for (i, c) in s.char_indices() {
        if units > 0 && units + c.len_utf16() > max {
            parts.push(&s[start..i]);
            (start, units) = (i, 0);
        }
        units += c.len_utf16();
    }
    parts.push(&s[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_parts_within_the_limit() {
        // `€` is one UTF-16 unit in three bytes, `𝄞` is two units in four bytes.
        let s = "ab€𝄞cd".repeat(100);
        for max in [2, 3, 5, 1280] {
            let parts = split(&s, max);
            assert_eq!(parts.concat(), s);
            assert!(parts.iter().all(|p| !p.is_empty() && p.encode_utf16().count() <= max));
        }
    }

    #[test]
    fn short_secret_is_one_part() {
        assert_eq!(split("{}", PART_LEN), vec!["{}"]);
    }

    /// Round trip through the real Credential Manager with a jar several credentials long.
    #[cfg(windows)]
    #[test]
    fn long_session_round_trips_through_credential_manager() {
        use mostik_yandex::session::{SESSION_COOKIE, StoredCookie};

        let jar = |count: usize| CookieJar {
            cookies: (0..count)
                .map(|i| StoredCookie {
                    name: if i == 0 {
                        SESSION_COOKIE.into()
                    } else {
                        format!("cookie{i}")
                    },
                    value: "v".repeat(200),
                    domain: ".yandex.ru".into(),
                })
                .collect(),
        };
        let service = format!("{SERVICE}.test.{}", std::process::id());

        let long = jar(30);
        save_to(&service, &long).unwrap();
        assert_eq!(load_from(&service), Some(long));

        // A shorter session must not pick up the leftover parts of the longer one.
        let short = jar(2);
        save_to(&service, &short).unwrap();
        assert_eq!(load_from(&service), Some(short));

        delete_parts(&service, 0).unwrap();
        assert_eq!(load_from(&service), None);
    }
}
