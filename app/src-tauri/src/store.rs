//! Session cookies are kept in the Secret Service (KWallet / GNOME Keyring) via `keyring`.

use mostik_yandex::session::CookieJar;

const SERVICE: &str = "dev.mostik.client";
const ACCOUNT: &str = "yandex-session";

fn entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT)
}

/// Saved session, if there is one and it still has the session cookie.
pub fn load() -> Option<CookieJar> {
    let secret = entry().ok()?.get_password().ok()?;
    serde_json::from_str::<CookieJar>(&secret)
        .ok()
        .filter(CookieJar::has_session)
}

/// Saves the session in the password store.
pub fn save(jar: &CookieJar) -> Result<(), String> {
    let secret = serde_json::to_string(jar).map_err(|e| e.to_string())?;
    entry()
        .and_then(|e| e.set_password(&secret))
        .map_err(|e| format!("could not save the session to the password store: {e}"))
}

/// Forgets the saved session.
pub fn delete() {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
}
