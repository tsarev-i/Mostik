//! Connection to Telemost: signing in and out, connecting, reconnecting after a close, and the
//! snapshot a connection starts with.
//!
//! Connection diagnostics, in a debug build or with [`PROBE_ENV`] set: every connection writes
//! `probe.json`, and the first socket events go to `probe-events.jsonl` (in the app data dir).
//! They hold only structure (keys and types) and identifier patterns: no message text, names
//! or tokens.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use mostik_core::{ChatKind, Presence, Snapshot, User, UserId};
use mostik_yandex::convert::{self, LiveEvent};
use mostik_yandex::diag::{id_pattern, shape};
use mostik_yandex::identity;
use mostik_yandex::passport;
use mostik_yandex::session::CookieJar;
use mostik_yandex::web::{WebClient, WebError};
use mostik_yandex::wire::{self, request_id};
use mostik_yandex::xiva_client::{XivaClient, XivaError};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use super::events::handle_incoming;
use super::presence::start_heartbeat;
use super::{AppState, Boot, EVENT, Live, SIGNED_OUT_EVENT, SNAPSHOT_EVENT, Shared, fill_profiles, service, text};
use crate::{call, login, store};

/// Turns on the connection diagnostics (`probe*.json`, `probe-events.jsonl`) in a release build.
const PROBE_ENV: &str = "MOSTIK_PROBE";

/// Which sign-in the backend works for. Signing in and out starts a new generation, and work
/// begun for an older one (a reconnect, a cookie refresh, a push of its connection) is dropped
/// instead of signing the user back in or writing the old cookies back.
#[derive(Default)]
pub(super) struct Session {
    generation: AtomicU64,
    /// Held while the password store is written or cleared, so that a save checked against the
    /// generation cannot land after the sign-out has deleted the session.
    store: StdMutex<()>,
}

impl Session {
    fn current(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn is_current(&self, generation: u64) -> bool {
        self.current() == generation
    }

    fn lock_store(&self) -> std::sync::MutexGuard<'_, ()> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts a new generation (a sign-in or a sign-out) and returns it.
    fn renew(&self) -> u64 {
        let _store = self.lock_store();
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Saves the cookies of session `generation`, unless it has ended.
    fn save(&self, generation: u64, jar: &CookieJar) -> Result<(), String> {
        let _store = self.lock_store();
        if !self.is_current(generation) {
            return Err("the session has ended".into());
        }
        store::save(jar)
    }

    /// Deletes the saved session, unless a newer one than `generation` has replaced it.
    fn delete(&self, generation: u64) -> Result<(), String> {
        let _store = self.lock_store();
        if !self.is_current(generation) {
            return Ok(());
        }
        store::delete()
    }
}

/// Makes `live` the connection and starts its work: the heartbeat (once per run) and the service
/// chats, which also catches up on their changes made while offline. `false` when session
/// `generation` has ended meanwhile.
async fn install(app: &AppHandle, state: &AppState, generation: u64, live: Live) -> bool {
    let live = Arc::new(live);
    {
        let mut slot = state.live.lock().await;
        if !state.session.is_current(generation) {
            return false;
        }
        *slot = Some(live.clone());
    }
    start_heartbeat(app);
    service::start(app.clone(), live);
    true
}

/// The login screen's reason when Telemost no longer accepts the saved session.
fn session_expired(app: &AppHandle) -> &'static str {
    text(
        app,
        "Сессия Яндекс ID недействительна, войдите снова.",
        "Your Yandex ID session has expired. Please sign in again.",
    )
}

#[tauri::command]
pub async fn bootstrap(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Boot, String> {
    let generation = state.session.current();
    match store::load() {
        None => Ok(Boot::NeedLogin { reason: None }),
        Some(jar) => connect(&app, &state, jar, false, generation).await,
    }
}

#[tauri::command]
pub async fn login(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Boot, String> {
    let rx = login::open(&app)?;
    let jar = rx.await.map_err(|_| {
        text(
            &app,
            "Вход отменён: окно Яндекс ID закрыто.",
            "Sign-in cancelled: the Yandex ID window was closed.",
        )
        .to_owned()
    })?;
    let generation = state.session.renew();
    state.session.save(generation, &jar)?;
    connect(&app, &state, jar, true, generation).await
}

/// "Sign out": ends the Yandex ID session on the server, as the Telemost web client does, then
/// forgets it here: the password store entry, the connection, the call and what the account
/// left on disk. The app is signed out in any case; an error says what did not work out, and
/// the UI shows it on the login screen.
#[tauri::command]
pub async fn logout(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // From here on nothing of the old session is saved again or reconnected.
    let generation = state.session.renew();
    let live = state.live.lock().await.take();
    // It waits for the call browser to go: not on an async worker.
    let closing = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || call::shutdown(&closing)).await;
    crate::ring::forget(&app);
    let jar = live.as_ref().map(|l| l.web.jar().clone()).or_else(store::load);
    drop(live);
    let revoked = match jar {
        Some(jar) => passport::logout(&jar, &identity::user_agent(identity::BROWSER_UA))
            .await
            .map_err(|e| {
                format!(
                    "{}: {e}",
                    text(
                        &app,
                        "Яндекс ID не завершил сессию, она может оставаться действующей",
                        "Yandex ID did not end the session, it may still be valid"
                    )
                )
            }),
        None => Ok(()),
    };
    let deleted = state.session.delete(generation).map_err(|e| {
        format!(
            "{}: {e}",
            text(
                &app,
                "Сессия осталась в хранилище паролей, при следующем запуске вход будет выполнен снова",
                "The session stays in the password store and the next start signs in again"
            )
        )
    });
    forget_local_data(&app, &state).await;
    // A session left in the password store comes back at the next start: that one matters most.
    deleted.and(revoked)
}

/// Removes what the signed-out account left on this computer: the diagnostic probes (they name
/// its cookies and chats), the custom emoji cache, the files allowed for sending, and the
/// cookies and caches of the main window. The UI clears its own `localStorage` keys.
async fn forget_local_data(app: &AppHandle, state: &AppState) {
    if let Ok(dir) = app.path().app_data_dir()
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("probe") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    service::forget_account(app).await;
    if let Ok(mut allowed) = state.allowed_files.lock() {
        allowed.clear();
    }
    if let Ok(mut kept) = state.kept_files.lock() {
        kept.clear();
    }
    if let Some(main) = app.get_webview_window("main") {
        crate::webview::forget_site_data(&main, crate::webview::SiteData::CookiesAndCaches);
    }
}

fn err_json(e: impl ToString) -> Value {
    json!({ "error": e.to_string() })
}

/// Whether the connection diagnostics are on: in a debug build, or with `MOSTIK_PROBE=1`.
pub(crate) fn probing() -> bool {
    static ON: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| cfg!(debug_assertions) || std::env::var(PROBE_ENV).is_ok_and(|v| v == "1"));
    *ON
}

/// Writes diagnostics file `name` into the app data dir, when they are on ([`probing`]).
pub(crate) fn write_probe(app: &AppHandle, name: &str, probe: &Value) {
    if !probing() {
        return;
    }
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(name), serde_json::to_string_pretty(probe).unwrap_or_default());
    }
}

/// `signed_in`: the session has just come from the login window, so a rejection means the
/// sign-in did not go through rather than an expired session. `generation`: see [`Session`].
async fn connect(
    app: &AppHandle,
    state: &AppState,
    jar: CookieJar,
    signed_in: bool,
    generation: u64,
) -> Result<Boot, String> {
    match connect_probed(app, state, jar, generation).await {
        Ok((snapshot, live)) => {
            if !install(app, state, generation, live).await {
                return Ok(Boot::NeedLogin { reason: None });
            }
            Ok(Boot::Ready {
                snapshot: Box::new(snapshot),
            })
        }
        Err(ConnectError::Unauthorized) => {
            if let Err(e) = state.session.delete(generation) {
                eprintln!("{e}");
            }
            let reason = if signed_in {
                text(
                    app,
                    "Телемост не принял вход через Яндекс ID. Попробуйте войти ещё раз.",
                    "Telemost did not accept the Yandex ID sign-in. Please try again.",
                )
            } else {
                session_expired(app)
            };
            Ok(Boot::NeedLogin {
                reason: Some(reason.into()),
            })
        }
        Err(ConnectError::Other(e)) => Err(e),
    }
}

enum ConnectError {
    Unauthorized,
    Other(String),
}

impl From<WebError> for ConnectError {
    fn from(e: WebError) -> Self {
        match e {
            WebError::Unauthorized => ConnectError::Unauthorized,
            other => ConnectError::Other(other.to_string()),
        }
    }
}

/// Reconnects session `generation` after its connection closed. One loop at a time: the closes
/// that come while it runs are its own business.
pub(super) async fn reconnect_loop(app: AppHandle, mut generation: u64) {
    let state = app.state::<AppState>();
    loop {
        if state.reconnecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let reconnected = reconnect(&app, &state, generation).await;
        state.reconnecting.store(false, Ordering::SeqCst);
        // A connection that closed while the flag turned its own reconnection away would
        // otherwise stay dead: the new one closing at once, or that of a sign-in that came
        // while this loop still worked for the session before.
        let dead = state.live.lock().await.as_ref().is_some_and(|l| l.xiva.is_closed());
        let ended = !state.session.is_current(generation);
        if !(dead && (reconnected || ended)) {
            return;
        }
        generation = state.session.current();
        eprintln!("reconnect: the connection closed while reconnecting");
        let _ = app.emit(EVENT, &LiveEvent::Connection { online: false });
    }
}

/// Connects session `generation` again and again, with growing pauses, until a connection is
/// up (`true`), there is no session to connect with or it has ended (`false`).
async fn reconnect(app: &AppHandle, state: &AppState, generation: u64) -> bool {
    let mut attempt = 0;
    loop {
        tokio::time::sleep(reconnect_pause(attempt, random())).await;
        attempt = attempt.saturating_add(1);
        // Signed out meanwhile: nothing to restore.
        if !state.session.is_current(generation) {
            return false;
        }
        let Some(jar) = store::load() else { return false };
        match connect_probed(app, state, jar, generation).await {
            // Its close went to a reconnection that this loop turned away: try again.
            Ok((_, live)) if live.xiva.is_closed() => eprintln!("reconnect: the new connection closed at once"),
            Ok((snapshot, live)) => {
                if !install(app, state, generation, live).await {
                    return false;
                }
                let _ = app.emit(EVENT, &LiveEvent::Connection { online: true });
                let _ = app.emit(SNAPSHOT_EVENT, &snapshot);
                return true;
            }
            Err(ConnectError::Unauthorized) => {
                if !state.session.is_current(generation) {
                    return false;
                }
                if let Err(e) = state.session.delete(generation) {
                    eprintln!("{e}");
                }
                call::close(app);
                *state.live.lock().await = None;
                let _ = app.emit(SIGNED_OUT_EVENT, session_expired(app));
                return false;
            }
            Err(ConnectError::Other(e)) => eprintln!("reconnect failed: {e}"),
        }
    }
}

/// The pause before reconnection attempt `attempt` (from 0): 2, 4, 8 … 60 s, each stretched by
/// up to a half at random (`random` is any number), so that clients cut off together (a
/// Telemost restart, a network change at an office) do not come back all at once.
fn reconnect_pause(attempt: u32, random: u64) -> Duration {
    let base = 2_000u64.saturating_mul(1 << attempt.min(5)).min(60_000);
    Duration::from_millis(base + random % (base / 2 + 1))
}

/// A random number, for the jitter of [`reconnect_pause`]: std's hasher keys are random.
fn random() -> u64 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one(std::time::Instant::now())
}

/// Chat count and chat ID patterns of a `history` response, for the probe.
pub(super) fn id_patterns(v: &Value) -> Value {
    let chats = v.get("Chats").and_then(Value::as_array);
    let patterns: BTreeSet<String> = chats
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("ChatId").and_then(Value::as_str))
        .map(id_pattern)
        .collect();
    json!({ "chats": chats.map_or(0, Vec::len), "id_patterns": patterns })
}

/// `whoami` over the socket, tried a few times: an answer without `UserInfo` counts as a failure.
/// Every attempt goes to the probe; `Null` if none succeeded.
async fn whoami(xiva: &XivaClient, probe: &mut Value) -> Value {
    let mut attempts = Vec::new();
    for _ in 0..3 {
        let who = xiva.call("whoami", &json!({ "RequestId": request_id() }), 10).await;
        attempts.push(who.as_ref().map(shape).unwrap_or_else(err_json));
        match who {
            Ok(v) if v.pointer("/UserInfo/Guid").and_then(Value::as_str).is_some() => {
                probe["ws_whoami"] = json!(attempts);
                return v;
            }
            Err(XivaError::Closed) => break,
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    probe["ws_whoami"] = json!(attempts);
    Value::Null
}

/// [`connect_inner`], with what it found out written to `probe.json` (see [`probing`]).
async fn connect_probed(
    app: &AppHandle,
    state: &AppState,
    jar: CookieJar,
    generation: u64,
) -> Result<(Snapshot, Live), ConnectError> {
    let mut probe = json!({ "client": identity::ua_suffix() });
    let result = connect_inner(app, state, jar, generation, &mut probe).await;
    write_probe(app, "probe.json", &probe);
    result
}

async fn connect_inner(
    app: &AppHandle,
    state: &AppState,
    jar: CookieJar,
    generation: u64,
    probe: &mut Value,
) -> Result<(Snapshot, Live), ConnectError> {
    let ua = identity::user_agent(identity::BROWSER_UA);
    probe["cookies"] = json!(
        jar.cookies
            .iter()
            .map(|c| format!("{}@{}", c.name, c.domain))
            .collect::<Vec<_>>()
    );
    let saved = jar.clone();
    let mut web = WebClient::new(jar, ua.clone())?;

    let page = web.load_page().await;
    probe["page"] = match &page {
        Ok(p) => json!({ "puid_len": p.puid.len(), "service_id": p.service_id.is_some(), "logged_in": p.logged_in }),
        Err(e) => err_json(e),
    };
    probe["page"]["hops"] = json!(web.page_hops);
    let page = page?;
    // The page refreshed some cookies on its redirects: keep them for the next start.
    if web.jar() != &saved {
        probe["page"]["jar_saved"] = json!(state.session.save(generation, web.jar()).is_ok());
    }

    let shared: Arc<StdMutex<Shared>> = Arc::default();
    let (handler_app, handler_shared, diag) = (app.clone(), shared.clone(), state.diag_events.clone());
    let xiva = XivaClient::connect(web.jar(), &ua, &page.puid, move |incoming| {
        // The connection of a session that has ended (signed out) has nothing more to say.
        if handler_app.state::<AppState>().session.is_current(generation) {
            handle_incoming(&handler_app, &handler_shared, &diag, generation, incoming)
        }
    })
    .await
    .map_err(|e| {
        probe["xiva"] = err_json(&e);
        ConnectError::Other(format!("Could not connect to Telemost: {e}"))
    })?;
    probe["xiva"] = json!({ "connected": true });

    // Xiva may ignore requests sent before it has subscribed the socket, so wait for that first.
    probe["subscription"] = json!(xiva.wait_subscription(Duration::from_secs(5)).await.is_some());
    let who = whoami(&xiva, probe).await;

    let list = xiva.call("history", &wire::chat_list(0), 40).await;
    probe["history"] = list.as_ref().map(id_patterns).unwrap_or_else(err_json);
    let list = list.map_err(|e| ConnectError::Other(format!("Telemost did not return the chat list: {e}")))?;

    // Without `whoami` every direct chat would show its first member, often yourself: take the
    // GUID from the chat IDs and the name, photo and organization from the directory instead.
    let (who, known_me) = match who.pointer("/UserInfo/Guid").and_then(Value::as_str).map(str::to_owned) {
        Some(guid) => (who, Some(UserId(guid))),
        None => match convert::own_guid(&list) {
            Some(me) => {
                let profile = web.rpc("get_users_data", json!({ "guids": [me.0] })).await;
                probe["me_fallback"] = profile.as_ref().map(shape).unwrap_or_else(err_json);
                let profile = profile
                    .ok()
                    .and_then(|v| convert::profiles(&v).into_iter().find(|p| p.id == me));
                (convert::whoami_from_profile(&me, profile.as_ref()), Some(me))
            }
            None => {
                probe["me_fallback"] = json!("unknown");
                (Value::Null, None)
            }
        },
    };
    if let Some(me) = &known_me
        && let Ok(mut sh) = shared.lock()
    {
        // Known before the rest of the snapshot, so pushes arriving meanwhile recognise our own messages.
        sh.me = Some(me.clone());
    }
    let me_name = who
        .pointer("/UserInfo/DisplayName")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let organization = who
        .pointer("/UserInfo/OrganizationInfos/0/OrganizationName")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .unwrap_or("Telemost")
        .to_owned();

    let mut data = convert::from_history(&list, known_me.as_ref());
    let service_chats = match &known_me {
        Some(me) => service::detect(app, &mut web, &list, me).await,
        None => vec![],
    };
    probe["service_chats"] = json!(service_chats.len());
    // The chat with oneself, as in Slack: create it if it does not exist yet (the call is idempotent).
    if let Some(me) = &known_me
        && !data.chats.iter().any(|c| c.id == convert::notes_chat_id(me))
    {
        match web.rpc("create_private_chat", json!({ "guid": me.0 })).await {
            Ok(v) if v.get("status").and_then(Value::as_str) == Some("ok") => {
                data.chats.push(convert::notes_chat(me));
                probe["notes"] = json!("created");
            }
            Ok(v) => probe["notes"] = json!({ "code": v.pointer("/data/code") }),
            Err(e) => probe["notes"] = err_json(e),
        }
    } else {
        probe["notes"] = json!("exists");
    }
    let last = std::mem::take(&mut data.last);
    // The snapshot and the connection need an ID even for a user nothing named: one that
    // matches nobody.
    let me_id = known_me.clone().unwrap_or_else(|| UserId("me".into()));
    let me = User {
        presence: Presence::Online,
        avatar: who
            .pointer("/UserInfo/AvatarId")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty())
            .map(str::to_owned),
        ..User::named(me_id.clone(), me_name.unwrap_or_else(|| "You".into()))
    };
    let mut snapshot = convert::snapshot(me, organization, data);
    snapshot.organization_avatar = who
        .pointer("/UserInfo/OrganizationInfos/0/AvatarId")
        .and_then(Value::as_str)
        .filter(|a| !a.is_empty())
        .map(str::to_owned);
    snapshot.organizations = who
        .pointer("/UserInfo/OrganizationInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| {
            Some(mostik_core::Organization {
                id: o.get("OrganizationId").and_then(convert::uint)?,
                name: o
                    .get("OrganizationName")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                avatar: o
                    .get("AvatarId")
                    .and_then(Value::as_str)
                    .filter(|a| !a.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect();
    let filled = fill_profiles(&web, &mut snapshot.users).await;
    probe["profiles_filled"] = json!(filled);
    match web.buckets().await {
        Ok(b) => {
            convert::apply_buckets(&mut snapshot, &b);
            probe["buckets"] = json!(b.get("status"));
        }
        Err(e) => probe["buckets"] = err_json(e),
    }
    let service_chats = service::hide(&mut snapshot, service_chats);
    if let Ok(mut sh) = shared.lock() {
        sh.me = known_me.clone();
        sh.paused = snapshot.paused;
        sh.service = service_chats.iter().map(|s| s.id.clone()).collect();
        sh.service_chats = service_chats;
        sh.orgs = snapshot.organizations.iter().map(|o| o.id).collect();
        // Pushes that arrived while connecting may already be newer than the history.
        for (chat, (ts, seq)) in last {
            let entry = sh.last.entry(chat).or_default();
            if ts > entry.0 {
                *entry = (ts, seq.max(entry.1));
            }
        }
        for ch in &snapshot.chats {
            if ch.muted {
                sh.muted.insert(ch.id.0.clone(), ch.notify_level());
            }
            let title = match ch.kind {
                ChatKind::Group { .. } | ChatKind::Broadcast => mostik_core::group_label(&ch.name),
                _ => mostik_core::chat_title(&snapshot, ch),
            };
            sh.titles.insert(ch.id.0.clone(), title);
        }
        sh.learn_names(&snapshot.users);
        sh.channels = snapshot.mention_channels();
        sh.ready = true;
    }
    probe["snapshot"] = json!({
        "chats": snapshot.chats.len(),
        "users": snapshot.users.len(),
        "messages": snapshot.messages.len(),
        "direct": snapshot.chats.iter().filter(|c| matches!(c.kind, ChatKind::Direct { .. })).count(),
        "me_known": known_me.is_some(),
        "with_avatar": snapshot.users.iter().filter(|u| u.avatar.is_some()).count(),
        "org_avatar": snapshot.organization_avatar.is_some(),
    });
    Ok((
        snapshot,
        Live {
            files: StdMutex::default(),
            downloads: StdMutex::default(),
            web,
            xiva,
            me: me_id,
            user_agent: ua,
            service_id: page.service_id,
            shared,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::reconnect_pause;
    use std::time::Duration;

    #[test]
    fn reconnect_pauses_grow_with_jitter() {
        let ms = |attempt, random| reconnect_pause(attempt, random).as_millis();
        assert_eq!(
            [0, 1, 2, 3, 4, 5, 40].map(|a| ms(a, 0)),
            [2000, 4000, 8000, 16000, 32000, 60000, 60000]
        );
        // Up to a half more, at random.
        assert_eq!(ms(0, 1000), 3000);
        assert_eq!(ms(0, 1001), 2000);
        assert_eq!(ms(5, u64::MAX), 60000 + u128::from(u64::MAX % 30001));
        assert!(reconnect_pause(40, u64::MAX) <= Duration::from_secs(90));
    }
}
