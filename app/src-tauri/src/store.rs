//! Session cookies are kept in the system password store via `keyring`: the Secret Service
//! (KWallet / GNOME Keyring) on Linux, the Credential Manager on Windows, the login keychain on macOS.
//!
//! The Credential Manager holds at most 1280 UTF-16 characters per credential, less than a typical
//! cookie jar, so there the secret is split into parts: `yandex-session`, `yandex-session.1`, …
//! On Linux and macOS it always fits into the first one.

use mostik_yandex::session::CookieJar;

const SERVICE: &str = "dev.mostik.client";
const ACCOUNT: &str = "yandex-session";

/// Longest part in UTF-16 code units: the Credential Manager keeps a password as UTF-16
/// in a blob of at most 2560 bytes.
#[cfg(windows)]
const PART_LEN: usize = 1280;
#[cfg(not(windows))]
const PART_LEN: usize = usize::MAX;

/// Saved session, if there is one and it still has the session cookie.
pub fn load() -> Option<CookieJar> {
    load_from(SERVICE)
}

/// Saves the session in the password store.
pub fn save(jar: &CookieJar) -> Result<(), String> {
    save_to(SERVICE, jar)
}

/// Forgets the saved session.
pub fn delete() {
    delete_parts(SERVICE, 0);
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
    delete_parts(service, parts.len());
    Ok(())
}

/// Deletes the parts from `first` on, up to the first missing one.
fn delete_parts(service: &str, first: usize) {
    for part in first.. {
        if entry(service, part).and_then(|e| e.delete_credential()).is_err() {
            break;
        }
    }
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

        delete_parts(&service, 0);
        assert_eq!(load_from(&service), None);
    }
}
