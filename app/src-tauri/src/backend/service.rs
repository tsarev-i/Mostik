//! The Mostik service chat of an organization and its custom emoji (see
//! [`mostik_core::custom_emoji`]).
//!
//! The chat is found in the chat list (by name, confirmed by the marker in its description) or by
//! its alias, [`ce::service_alias`] of the organization, which Mostik then joins: Telemost search
//! finds only the chats the user is in. When there is none, the first emoji added creates it. It never reaches the UI: [`hide`] takes it out of the snapshot and [`swallows`] out of the
//! live events. Mostik mutes it in Telemost and keeps it read, so the official client does not
//! bother anyone with it either.
//!
//! The sets and the emoji images are cached on disk: a start shows them at once and reads the chat
//! again only when its last message or edit has changed.

use std::path::PathBuf;

use mostik_core::custom_emoji::{
    self as ce, EmojiSet, MAX_UPLOAD, SERVICE_CHAT_NAME, SERVICE_DESCRIPTION, SERVICE_MARKER, UPLOAD_SIDE,
};
use serde::Deserialize;

use super::*;

/// The custom emoji sets changed: the payload is the new list.
pub const EMOJI_EVENT: &str = "mostik://emoji";

/// A service chat the user is in, with what tells whether its cached set is still current.
#[derive(Clone, Debug)]
pub struct ServiceChat {
    pub id: String,
    pub organizations: Vec<u64>,
    last_ts: u64,
    last_edit_ts: u64,
    last_seq: u64,
    seen_seq: u64,
    muted: bool,
    /// In the chat list. A known chat missing from it is read again in any case: the user may
    /// have left it.
    listed: bool,
}

/// A set on disk with the chat state it was read at.
#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    set: EmojiSet,
    last_ts: u64,
    last_edit_ts: u64,
}

/// Fields missing on disk (written by an older version) take their defaults.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Disk {
    /// The service chat scheme the cache is for ([`SERVICE_MARKER`]): a cache of an older one
    /// (another chat name and alias) is dropped.
    marker: String,
    /// Whose sets these are: another account starts afresh.
    me: String,
    sets: Vec<Cached>,
    /// When the aliases were last looked up (ms since the epoch).
    looked_ms: u64,
    /// Service chats that hold the alias of their organization: of several service chats of one
    /// organization every client uses that one.
    aliased: BTreeSet<String>,
}

impl Disk {
    /// An empty cache of the current scheme.
    fn fresh() -> Self {
        Self {
            marker: SERVICE_MARKER.to_owned(),
            ..Self::default()
        }
    }

    /// The cache from its file; an unreadable one or one of an older scheme starts afresh.
    fn load(bytes: &[u8]) -> Self {
        serde_json::from_slice::<Self>(bytes)
            .ok()
            .filter(|d| d.marker == SERVICE_MARKER)
            .unwrap_or_else(Self::fresh)
    }
}

/// Service chat state that outlives a connection.
#[derive(Default)]
pub struct State {
    disk: StdMutex<Option<Disk>>,
    /// Chats whose re-read is already scheduled: a burst of pushes reads a chat once.
    scheduled: StdMutex<BTreeSet<String>>,
    /// The last message number marked read per chat.
    marked: StdMutex<BTreeMap<String, u64>>,
    /// One read or write of the service chat at a time.
    busy: Mutex<()>,
}

/// How often the alias of an organization is looked up while its service chat is unknown.
const LOOK_EVERY_MS: u64 = 3600 * 1000;
/// History page for reading the service chat, and the most pages read.
const PAGE: u32 = 50;
const MAX_PAGES: usize = 100;
/// Emoji images downloaded at the same time.
const DOWNLOADS: usize = 4;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("emoji"))
}

/// Runs `f` on the disk cache, loading it first.
fn with_disk<R>(app: &AppHandle, f: impl FnOnce(&mut Disk) -> R) -> R {
    let state = app.state::<AppState>();
    let mut guard = state.service.disk.lock().unwrap_or_else(|e| e.into_inner());
    let disk = guard.get_or_insert_with(|| {
        dir(app)
            .and_then(|d| std::fs::read(d.join("sets.json")).ok())
            .map_or_else(Disk::fresh, |b| Disk::load(&b))
    });
    f(disk)
}

fn save(app: &AppHandle) {
    let Some(dir) = dir(app) else { return };
    let json = with_disk(app, |d| serde_json::to_vec(d).ok());
    if let Some(json) = json
        && std::fs::create_dir_all(&dir).is_ok()
    {
        let tmp = dir.join("sets.json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, dir.join("sets.json"));
        }
    }
}

/// The sets to use, one per organization: of several service chats of one organization (two
/// people created one at the same time), every client takes the same one, the one with the alias.
pub fn sets(app: &AppHandle) -> Vec<EmojiSet> {
    let mut all: Vec<(bool, EmojiSet)> = with_disk(app, |d| {
        d.sets
            .iter()
            .map(|c| (!d.aliased.contains(&c.set.chat_id.0), c.set.clone()))
            .collect()
    });
    all.sort_by(|a, b| (a.0, &a.1.chat_id.0).cmp(&(b.0, &b.1.chat_id.0)));
    let mut out: Vec<EmojiSet> = Vec::new();
    for (_, set) in all {
        let covered = out.iter().any(|o| {
            o.organizations.iter().any(|x| set.organizations.contains(x))
                || (o.organizations.is_empty() && set.organizations.is_empty())
        });
        if !covered {
            out.push(set);
        }
    }
    out
}

fn emit(app: &AppHandle) {
    let _ = app.emit(EMOJI_EVENT, sets(app));
}

/// Whether a chat is a service chat: its name and the marker in its description.
pub fn is_service_chat(chat: &mostik_core::Chat) -> bool {
    chat.name == SERVICE_CHAT_NAME && chat.description.as_deref().is_some_and(|d| d.contains(SERVICE_MARKER))
}

fn u64_list(v: Option<&Value>) -> Vec<u64> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect()
}

/// The service chats in the chat list: by name or by an ID seen before; a new one is confirmed
/// by the marker in its description (`get_chats_info`, which the list does not carry).
pub async fn detect(app: &AppHandle, web: &mut WebClient, list: &Value, me: &UserId) -> Vec<ServiceChat> {
    let known: BTreeSet<String> = with_disk(app, |d| {
        if d.me != me.0 {
            *d = Disk {
                me: me.0.clone(),
                ..Disk::fresh()
            };
        }
        d.sets.iter().map(|c| c.set.chat_id.0.clone()).collect()
    });
    let mut found: Vec<ServiceChat> = list
        .get("Chats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = c.get("ChatId")?.as_str()?;
            let named = c.pointer("/ChatInfo/Name").and_then(Value::as_str) == Some(SERVICE_CHAT_NAME);
            (named || known.contains(id)).then(|| ServiceChat {
                id: id.to_owned(),
                organizations: u64_list(c.pointer("/ChatInfo/OrganizationIds")),
                last_ts: c.get("LastTsMcs").and_then(Value::as_u64).unwrap_or(0),
                last_edit_ts: c.get("LastEditTsMcs").and_then(Value::as_u64).unwrap_or(0),
                last_seq: c.get("LastSeqNo").and_then(Value::as_u64).unwrap_or(0),
                seen_seq: c.get("LastSeenByMeSeqNo").and_then(Value::as_u64).unwrap_or(0),
                muted: false,
                listed: true,
            })
        })
        .collect();
    let unconfirmed: Vec<String> = found
        .iter()
        .filter(|s| !known.contains(&s.id))
        .map(|s| s.id.clone())
        .collect();
    if !unconfirmed.is_empty() {
        let confirmed: BTreeSet<String> = match web.rpc("get_chats_info", json!({ "chat_ids": unconfirmed })).await {
            Ok(v) => marked_chats(&v).into_iter().map(|(id, _)| id).collect(),
            Err(e) => {
                eprintln!("service chat: {e}");
                BTreeSet::new()
            }
        };
        found.retain(|s| known.contains(&s.id) || confirmed.contains(&s.id));
    }
    eprintln!(
        "service chats in the list: {} of {} chats ({} known before)",
        found.len(),
        list.get("Chats").and_then(Value::as_array).map_or(0, Vec::len),
        known.len()
    );
    // Right after a change Telemost may leave a chat out of the list: a known one stays a
    // service chat, keeps its set and is read again.
    let missing: Vec<ServiceChat> = with_disk(app, |d| {
        d.sets
            .iter()
            .filter(|c| !found.iter().any(|f| f.id == c.set.chat_id.0))
            .map(|c| ServiceChat {
                id: c.set.chat_id.0.clone(),
                organizations: c.set.organizations.clone(),
                last_ts: 0,
                last_edit_ts: 0,
                last_seq: 0,
                seen_seq: 0,
                muted: true,
                listed: false,
            })
            .collect()
    });
    found.extend(missing);
    found
}

/// Service chats in a `get_chats_info` answer: `(chat_id, organization_ids)`.
fn marked_chats(v: &Value) -> Vec<(String, Vec<u64>)> {
    v.pointer("/data/chats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("name").and_then(Value::as_str) == Some(SERVICE_CHAT_NAME))
        .filter(|c| {
            c.get("description")
                .and_then(Value::as_str)
                .is_some_and(|d| d.contains(SERVICE_MARKER))
        })
        .filter_map(|c| {
            Some((
                c.get("chat_id")?.as_str()?.to_owned(),
                u64_list(c.get("organization_ids")),
            ))
        })
        .collect()
}

/// Takes the service chats out of the snapshot; returns them with their mute state from it.
pub fn hide(snapshot: &mut Snapshot, mut chats: Vec<ServiceChat>) -> Vec<ServiceChat> {
    for sc in &mut chats {
        if let Some(c) = snapshot.chats.iter().find(|c| c.id.0 == sc.id) {
            sc.muted = c.notify_level() == NotifyLevel::Nothing;
        }
    }
    let ids: BTreeSet<&str> = chats.iter().map(|s| s.id.as_str()).collect();
    snapshot.chats.retain(|c| !ids.contains(c.id.0.as_str()));
    snapshot.messages.retain(|m| !ids.contains(m.chat_id.0.as_str()));
    for sec in &mut snapshot.sections {
        sec.chats.retain(|c| !ids.contains(c.0.as_str()));
    }
    chats
}

/// The chat a live event is about.
fn event_chat(ev: &LiveEvent) -> Option<(&str, bool)> {
    Some(match ev {
        LiveEvent::Upsert { message, .. } => (message.chat_id.0.as_str(), message.thread_parent.is_none()),
        LiveEvent::Deleted {
            chat_id, thread_parent, ..
        } => (chat_id.0.as_str(), thread_parent.is_none()),
        LiveEvent::Typing { chat_id, .. }
        | LiveEvent::Reactions { chat_id, .. }
        | LiveEvent::Seen { chat_id }
        | LiveEvent::ThreadSeen { chat_id, .. }
        | LiveEvent::Pinned { chat_id, .. } => (chat_id.0.as_str(), false),
        LiveEvent::ChatAdded { chat, .. } => (chat.id.0.as_str(), false),
        _ => return None,
    })
}

/// Whether a live event belongs to a service chat: then the UI does not get it, and a new,
/// changed or deleted message there reads the chat again.
pub(super) fn swallows(app: &AppHandle, shared: &StdMutex<Shared>, ev: &LiveEvent) -> bool {
    let Some((chat, message)) = event_chat(ev) else {
        return false;
    };
    if !shared.lock().is_ok_and(|sh| sh.service.contains(chat)) {
        return false;
    }
    if message {
        schedule(app, chat.to_owned());
    }
    true
}

/// A chat the list did not have turned out to be a service chat (the user joined it elsewhere):
/// it is hidden from now on and read.
pub(super) fn adopt(app: &AppHandle, shared: &StdMutex<Shared>, chat: &mostik_core::Chat) {
    if let Ok(mut sh) = shared.lock() {
        sh.service.insert(chat.id.0.clone());
    }
    schedule(app, chat.id.0.clone());
}

/// Reads a service chat again a moment later.
fn schedule(app: &AppHandle, chat: String) {
    let state = app.state::<AppState>();
    if !state.service.scheduled.lock().is_ok_and(|mut s| s.insert(chat.clone())) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let state = app.state::<AppState>();
        if let Ok(mut s) = state.service.scheduled.lock() {
            s.remove(&chat);
        }
        let Ok(live) = live(&state).await else { return };
        let _busy = state.service.busy.lock().await;
        let orgs = organizations_of(&app, &chat);
        if let Err(e) = refresh(&app, &live, &chat, orgs).await {
            eprintln!("service chat {}: {e}", id_pattern(&chat));
        }
        emit(&app);
    });
}

fn organizations_of(app: &AppHandle, chat: &str) -> Vec<u64> {
    with_disk(app, |d| {
        d.sets
            .iter()
            .find(|c| c.set.chat_id.0 == chat)
            .map(|c| c.set.organizations.clone())
            .unwrap_or_default()
    })
}

/// After a connection: reads the service chats that changed, mutes them, keeps them read, looks
/// for one in organizations that have none and fetches the missing images.
pub fn start(app: AppHandle, live: Arc<Live>) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let _busy = state.service.busy.lock().await;
        let chats = live
            .shared
            .lock()
            .map(|sh| sh.service_chats.clone())
            .unwrap_or_default();
        if let Ok(mut marked) = state.service.marked.lock() {
            for sc in &chats {
                marked.insert(sc.id.clone(), sc.seen_seq);
            }
        }
        for sc in &chats {
            if !sc.organizations.is_empty() {
                with_disk(&app, |d| {
                    for c in d.sets.iter_mut().filter(|c| c.set.chat_id.0 == sc.id) {
                        c.set.organizations.clone_from(&sc.organizations);
                    }
                });
            }
            if !sc.muted {
                mute(&live, &sc.id).await;
            }
            let current = sc.listed
                && with_disk(&app, |d| {
                    d.sets.iter().any(|c| {
                        c.set.chat_id.0 == sc.id && c.last_ts == sc.last_ts && c.last_edit_ts == sc.last_edit_ts
                    })
                });
            if current {
                if sc.seen_seq < sc.last_seq {
                    mark_read(&app, &live, &sc.id, sc.last_ts, sc.last_seq).await;
                }
            } else if let Err(e) = refresh(&app, &live, &sc.id, sc.organizations.clone()).await {
                eprintln!("service chat {}: {e}", id_pattern(&sc.id));
            }
        }
        let orgs = live.shared.lock().map(|sh| sh.orgs.clone()).unwrap_or_default();
        let unknown = orgs.iter().any(|o| !has_aliased(&app, *o));
        let due = with_disk(&app, |d| now_ms().saturating_sub(d.looked_ms) > LOOK_EVERY_MS);
        if unknown && due {
            for (id, orgs) in discover(&app, &live).await {
                let _ = refresh(&app, &live, &id, orgs).await;
            }
        }
        download(&app, &live).await;
        save(&app);
        emit(&app);
    });
}

/// Whether the user is in the service chat that holds the alias of organization `org`.
fn has_aliased(app: &AppHandle, org: u64) -> bool {
    with_disk(app, |d| {
        d.sets
            .iter()
            .any(|c| c.set.organizations.contains(&org) && d.aliased.contains(&c.set.chat_id.0))
    })
}

/// Looks up the service chats of the user's organizations by their aliases and joins the ones the
/// user is not in yet. An organization whose alias is free gets it on the user's own service chat
/// there, if the user may change that chat.
async fn discover(app: &AppHandle, live: &Live) -> Vec<(String, Vec<u64>)> {
    with_disk(app, |d| d.looked_ms = now_ms());
    let orgs = live.shared.lock().map(|sh| sh.orgs.clone()).unwrap_or_default();
    let mut web = live.web.clone_light();
    let mut joined = vec![];
    for org in orgs {
        if has_aliased(app, org) {
            continue;
        }
        let Some(alias) = ce::service_alias(org) else { continue };
        let found = match web.rpc("get_chats_info", json!({ "alias": alias })).await {
            Ok(v) => aliased_chat(&v, org),
            Err(e) => {
                eprintln!("service chat alias: {e}");
                continue;
            }
        };
        let Some(AliasedChat {
            id,
            organizations: orgs,
            invite,
            joinable,
        }) = found
        else {
            name_own(app, &mut web, org, &alias).await;
            continue;
        };
        // Whether the user is in the chat is Telemost's word, not the disk's: a chat they left or
        // were removed from may still be cached.
        let cached = with_disk(app, |d| {
            d.aliased.insert(id.clone());
            d.sets.iter().any(|c| c.set.chat_id.0 == id)
        });
        eprintln!(
            "service chat by alias: {}, joinable: {joinable}, cached: {cached}",
            id_pattern(&id)
        );
        if !joinable && cached {
            continue;
        }
        if let Ok(mut sh) = live.shared.lock() {
            sh.service.insert(id.clone());
        }
        if joinable {
            // A public chat is joined by its invite, as by a link: `activate_chat` answers a
            // non-member with `no_permissions_for_this_chat_operation`.
            let join = match &invite {
                Some(hash) => web.rpc("invite", json!({ "invite": hash })).await,
                None => web.rpc("activate_chat", json!({ "chat_id": id })).await,
            };
            if let Err(e) = join.as_ref().map_err(ToString::to_string).and_then(rpc_ok) {
                with_disk(app, |d| d.aliased.remove(&id));
                eprintln!("service chat join {}: {e}", id_pattern(&id));
                continue;
            }
            eprintln!("service chat {}: joined", id_pattern(&id));
        }
        mute(live, &id).await;
        joined.push((id, orgs));
    }
    save(app);
    joined
}

/// A service chat found by its alias.
#[derive(Debug, PartialEq, Eq)]
struct AliasedChat {
    id: String,
    organizations: Vec<u64>,
    /// The invite hash to join it by, when the answer has one.
    invite: Option<String>,
    /// Telemost offers the user to join it (the `join` right): they are not in it.
    joinable: bool,
}

/// The service chat in a `get_chats_info {alias}` answer, if it is one of organization `org`.
/// An alias is global in Telemost, so the name, the marker and the organization are checked; the
/// preview of a chat the user is not in may lack the description or the organizations.
fn aliased_chat(v: &Value, org: u64) -> Option<AliasedChat> {
    let c = v.pointer("/data/chats/0")?;
    if c.get("name").and_then(Value::as_str) != Some(SERVICE_CHAT_NAME) {
        return None;
    }
    if let Some(d) = c.get("description").and_then(Value::as_str)
        && !d.contains(SERVICE_MARKER)
    {
        return None;
    }
    let orgs = u64_list(c.get("organization_ids"));
    if !orgs.is_empty() && !orgs.contains(&org) {
        return None;
    }
    Some(AliasedChat {
        id: c.get("chat_id")?.as_str()?.to_owned(),
        organizations: if orgs.is_empty() { vec![org] } else { orgs },
        invite: c.get("invite_hash").and_then(Value::as_str).map(str::to_owned),
        joinable: c
            .get("rights")
            .and_then(Value::as_array)
            .is_some_and(|r| r.iter().any(|x| x == "join")),
    })
}

/// Gives the alias of organization `org` to the user's service chat there (the first by ID, as
/// [`sets`] picks), when the user may change it.
async fn name_own(app: &AppHandle, web: &mut WebClient, org: u64, alias: &str) {
    let Some(id) = with_disk(app, |d| {
        d.sets
            .iter()
            .filter(|c| c.set.organizations.contains(&org))
            .map(|c| c.set.chat_id.0.clone())
            .min()
    }) else {
        return;
    };
    let info = match web.rpc("get_chats_info", json!({ "chat_ids": [id] })).await {
        Ok(v) => v.pointer("/data/chats/0").cloned().unwrap_or_default(),
        Err(e) => {
            eprintln!("service chat {}: {e}", id_pattern(&id));
            return;
        }
    };
    let may = info
        .get("rights")
        .and_then(Value::as_array)
        .is_some_and(|r| r.iter().any(|x| x == "change"));
    if !may {
        return;
    }
    let version = info.get("version").and_then(Value::as_u64).unwrap_or(0);
    if set_alias(web, &id, version, alias).await {
        with_disk(app, |d| d.aliased.insert(id));
    }
}

/// Sets the alias of a service chat, with its name, description and publicity, so nothing else
/// changes.
async fn set_alias(web: &mut WebClient, id: &str, version: u64, alias: &str) -> bool {
    let r = web
        .rpc(
            "set_chat_info",
            json!({ "chat_id": id, "version": version, "name": SERVICE_CHAT_NAME,
                "description": SERVICE_DESCRIPTION, "public": true, "alias": alias }),
        )
        .await;
    match r.as_ref().map_err(ToString::to_string).and_then(rpc_ok) {
        Ok(()) => {
            eprintln!("service chat {}: alias set", id_pattern(id));
            true
        }
        Err(e) => {
            eprintln!("service chat {} alias: {e}", id_pattern(id));
            false
        }
    }
}

/// Mutes a service chat in Telemost, so no client notifies about it.
async fn mute(live: &Live, chat: &str) {
    let id = chat.to_owned();
    let (mute, mute_mentions) = NotifyLevel::Nothing.flags();
    let r = update_bucket(live, "chat_mutings", move |v| {
        v[&id] = json!({ "mute": mute, "mute_mentions": mute_mentions })
    })
    .await;
    if let Err(e) = r {
        eprintln!("service chat mute: {e}");
    }
}

async fn mark_read(app: &AppHandle, live: &Live, chat: &str, ts: Timestamp, seq: u64) {
    let state = app.state::<AppState>();
    let done = state
        .service
        .marked
        .lock()
        .is_ok_and(|m| m.get(chat).is_some_and(|s| *s >= seq));
    if done || ts == 0 || seq == 0 {
        return;
    }
    if push(live, wire::seen_marker(chat, ts, seq)).await.is_ok()
        && let Ok(mut m) = state.service.marked.lock()
    {
        m.insert(chat.to_owned(), seq);
    }
}

/// Whether a `history` answer says the user is not in the chat (they left it or were removed).
fn not_member(v: &Value) -> bool {
    v.get("ErrorInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("Message").and_then(Value::as_str))
        .any(|m| m.contains("not in chat members"))
}

/// Drops a service chat the user is no longer in: its set and its alias mark go, and the alias is
/// looked up again at once, so the client joins the chat anew instead of keeping an empty set.
fn forget(app: &AppHandle, chat: &str) {
    with_disk(app, |d| {
        d.sets.retain(|c| c.set.chat_id.0 != chat);
        d.aliased.remove(chat);
        d.looked_ms = 0;
    });
    save(app);
    eprintln!("service chat {}: not a member, forgotten", id_pattern(chat));
}

/// Reads the whole service chat, rebuilds its set, keeps it on disk and marks the chat read.
async fn refresh(app: &AppHandle, live: &Live, chat: &str, mut orgs: Vec<u64>) -> Result<(), String> {
    let mut messages = Vec::new();
    let mut before = None;
    let (mut last_ts, mut last_edit_ts, mut last_seq) = (0, 0, 0);
    for page in 0..MAX_PAGES {
        let mut body = None;
        for attempt in 0..2 {
            match live
                .xiva
                .call("history", &HistoryRequest::page(chat, PAGE, before), HISTORY_TIMEOUT_S)
                .await
            {
                Ok(v) if not_member(&v) => {
                    forget(app, chat);
                    return Err("the user is not in the chat any more".to_owned());
                }
                Ok(v) if v.get("Chats").is_some() => {
                    body = Some(v);
                    break;
                }
                Ok(v) => return Err(format!("no history: {}", shape(&v))),
                Err(e) if attempt == 1 => return Err(e.to_string()),
                Err(_) => {}
            }
        }
        let Some(body) = body else { break };
        let c = &body["Chats"][0];
        if page == 0 {
            last_ts = c.get("LastTsMcs").and_then(Value::as_u64).unwrap_or(0);
            last_edit_ts = c.get("LastEditTsMcs").and_then(Value::as_u64).unwrap_or(0);
            last_seq = c.get("LastSeqNo").and_then(Value::as_u64).unwrap_or(0);
            let o = u64_list(c.pointer("/ChatInfo/OrganizationIds"));
            if !o.is_empty() {
                orgs = o;
            }
        }
        let raw: Vec<u64> = c
            .get("Messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| {
                m.get("ServerMessage")
                    .unwrap_or(m)
                    .pointer("/ServerMessageInfo/Timestamp")
                    .and_then(Value::as_u64)
            })
            .collect();
        messages.extend(convert::from_history(&body, Some(&live.me)).messages);
        match raw.iter().min() {
            Some(oldest) if raw.len() >= PAGE as usize => before = Some(*oldest),
            _ => break,
        }
    }
    let set = ce::build(&ChatId(chat.to_owned()), &orgs, &messages);
    eprintln!(
        "service chat {}: {} messages, {} emoji, {} taken names",
        id_pattern(chat),
        messages.len(),
        set.emoji.len(),
        set.taken.len()
    );
    with_disk(app, |d| {
        d.sets.retain(|c| c.set.chat_id.0 != chat);
        d.sets.push(Cached {
            set,
            last_ts,
            last_edit_ts,
        });
    });
    save(app);
    let last = messages.iter().max_by_key(|m| m.seq_no);
    let (ts, seq) = match last {
        _ if last_ts > 0 && last_seq > 0 => (last_ts, last_seq),
        Some(m) => (m.ts, m.seq_no),
        None => (0, 0),
    };
    mark_read(app, live, chat, ts, seq).await;
    download(app, live).await;
    Ok(())
}

/// File of an emoji image on disk.
fn file_path(app: &AppHandle, file_id: &str) -> Option<PathBuf> {
    let safe: String = file_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    dir(app).map(|d| d.join("files").join(safe))
}

/// Downloads the images of the emoji that are not on disk yet.
async fn download(app: &AppHandle, live: &Live) {
    use futures_util::StreamExt;
    let missing: Vec<String> = sets(app)
        .iter()
        .flat_map(|s| s.emoji.iter().chain(&s.taken))
        .map(|e| e.file_id.clone())
        .filter(|id| file_path(app, id).is_some_and(|p| !p.exists()))
        .collect();
    let jobs = missing.into_iter().map(|id| async move { fetch(app, live, &id).await });
    futures_util::stream::iter(jobs)
        .buffer_unordered(DOWNLOADS)
        .for_each(|_| async {})
        .await;
}

async fn fetch(app: &AppHandle, live: &Live, file_id: &str) -> Result<Vec<u8>, String> {
    let (bytes, _) = live.web.fetch_file(file_id, None).await.map_err(|e| e.to_string())?;
    if let Some(path) = file_path(app, file_id)
        && let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_ok()
    {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(tmp, &path);
        }
    }
    Ok(bytes)
}

/// The image type by its first bytes: the storage does not always say.
fn sniff(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [0xFF, 0xD8, ..] => "image/jpeg",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Serves an emoji image for `mostikfile://localhost/emoji/<file id>`: from disk, so it shows
/// even before the connection is up, else from the storage.
pub async fn serve(app: &AppHandle, file_id: &str) -> Result<(Vec<u8>, String), String> {
    if let Some(bytes) = file_path(app, file_id).and_then(|p| std::fs::read(p).ok()) {
        let kind = sniff(&bytes).to_owned();
        return Ok((bytes, kind));
    }
    let live = live(&app.state::<AppState>()).await?;
    let bytes = fetch(app, &live, file_id).await?;
    let kind = sniff(&bytes).to_owned();
    Ok((bytes, kind))
}

/// The custom emoji sets, from disk until the connection reads the chats.
#[tauri::command]
pub fn custom_emoji(app: AppHandle) -> Vec<EmojiSet> {
    sets(&app)
}

/// Picks an image for a new emoji.
#[tauri::command]
pub async fn pick_emoji_image(app: AppHandle) -> Result<Option<PendingFile>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Images", &["png", "gif", "jpg", "jpeg", "webp"])
        .pick_file(move |picked| {
            let _ = tx.send(picked);
        });
    let picked = rx.await.map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|f| f.into_path().ok()) else {
        return Ok(None);
    };
    Ok(allow_files(&app, &[path]).into_iter().next())
}

/// An image ready to post: bytes, file extension and MIME type, size.
struct Prepared {
    bytes: Vec<u8>,
    ext: &'static str,
    mime: &'static str,
    width: u32,
    height: u32,
}

#[derive(Debug)]
enum PrepareError {
    Unsupported,
    BigGif,
    Big,
}

/// A GIF goes as is when it is small enough (animations are not re-encoded); a still image is
/// scaled down into [`UPLOAD_SIDE`] and posted as PNG.
fn prepare(bytes: Vec<u8>) -> Result<Prepared, PrepareError> {
    use image::ImageFormat;
    let format = image::guess_format(&bytes).map_err(|_| PrepareError::Unsupported)?;
    if format == ImageFormat::Gif {
        if bytes.len() > MAX_UPLOAD {
            return Err(PrepareError::BigGif);
        }
        let size = imagesize::blob_size(&bytes).map_err(|_| PrepareError::Unsupported)?;
        return Ok(Prepared {
            bytes,
            ext: "gif",
            mime: "image/gif",
            width: size.width as u32,
            height: size.height as u32,
        });
    }
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP) {
        return Err(PrepareError::Unsupported);
    }
    let img = image::load_from_memory_with_format(&bytes, format).map_err(|_| PrepareError::Unsupported)?;
    let img = if img.width() > UPLOAD_SIDE || img.height() > UPLOAD_SIDE {
        img.resize(UPLOAD_SIDE, UPLOAD_SIDE, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)
        .map_err(|_| PrepareError::Unsupported)?;
    let bytes = out.into_inner();
    if bytes.len() > MAX_UPLOAD {
        return Err(PrepareError::Big);
    }
    Ok(Prepared {
        bytes,
        ext: "png",
        mime: "image/png",
        width: img.width(),
        height: img.height(),
    })
}

fn name_error(app: &AppHandle, name: &str) -> Option<String> {
    if ce::valid_name(name) {
        return None;
    }
    let standard = ce::is_standard_name(name);
    Some(
        if standard {
            text(
                app,
                "Так называется стандартный emoji, выберите другое имя.",
                "A standard emoji has this name, pick another one.",
            )
        } else {
            text(
                app,
                "Имя: латинские строчные буквы, цифры, «_» и «-», до 40 символов.",
                "Name: lowercase Latin letters, digits, “_” and “-”, up to 40 characters.",
            )
        }
        .to_owned(),
    )
}

fn taken_error(app: &AppHandle) -> String {
    text(app, "Это имя уже занято.", "This name is already taken.").to_owned()
}

/// The service chat for organization `org` to post to: a known one, one found among the public
/// chats, or a new one.
async fn chat_for(app: &AppHandle, live: &Live, org: Option<u64>) -> Result<(String, Vec<u64>), String> {
    if let Some(s) = ce::set_for(&sets(app), &[], org) {
        return Ok((s.chat_id.0.clone(), s.organizations.clone()));
    }
    // Someone may have created it since the last lookup.
    if let Some((id, orgs)) = discover(app, live).await.into_iter().next() {
        refresh(app, live, &id, orgs.clone()).await?;
        return Ok((id, orgs));
    }
    // The organization the user is in, or their only one.
    let org = org.or_else(|| {
        live.shared
            .lock()
            .ok()
            .and_then(|sh| (sh.orgs.len() == 1).then(|| sh.orgs[0]))
    });
    create(app, live, org).await
}

/// Creates the service chat: a public group of organization `org` with the marker in its
/// description, muted. Telemost puts a new group in no organization until it is given one.
async fn create(app: &AppHandle, live: &Live, org: Option<u64>) -> Result<(String, Vec<u64>), String> {
    let mut web = live.web.clone_light();
    let v = web
        .rpc(
            "create_chat",
            json!({ "name": SERVICE_CHAT_NAME, "members": [live.me.0], "channel": false, "public": true }),
        )
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&v)?;
    let id = v
        .pointer("/data/chat_id")
        .and_then(Value::as_str)
        .ok_or("Telemost did not return the chat ID")?
        .to_owned();
    // Hidden before any of its events arrive.
    if let Ok(mut sh) = live.shared.lock() {
        sh.service.insert(id.clone());
    }
    let version = v.pointer("/data/version").and_then(Value::as_u64).unwrap_or(0);
    let r = web
        .rpc(
            "set_chat_info",
            json!({ "chat_id": id, "version": version, "name": SERVICE_CHAT_NAME,
                "description": SERVICE_DESCRIPTION, "public": true }),
        )
        .await
        .map_err(|e| e.to_string())?;
    rpc_ok(&r)?;
    if let Some(org) = org {
        let r = web
            .rpc(
                "update_chat_organizations",
                json!({ "chat": id, "organization_id": org }),
            )
            .await;
        if let Err(e) = r.as_ref().map_err(ToString::to_string).and_then(rpc_ok) {
            eprintln!("service chat organization: {e}");
        }
    }
    mute(live, &id).await;
    let info = chat_info_raw(live, &id).await.unwrap_or_default();
    let orgs = u64_list(info.get("organization_ids"));
    // The alias lets colleagues find the chat; when another one took it a moment ago, every
    // client turns to that one.
    let version = info.get("version").and_then(Value::as_u64).unwrap_or(0);
    let alias = org.filter(|o| orgs.contains(o)).and_then(ce::service_alias);
    let aliased = match alias {
        Some(alias) => set_alias(&mut web, &id, version, &alias).await,
        None => false,
    };
    with_disk(app, |d| {
        if aliased {
            d.aliased.insert(id.clone());
        }
        d.sets.push(Cached {
            set: EmojiSet::empty(ChatId(id.clone()), orgs.clone()),
            last_ts: 0,
            last_edit_ts: 0,
        })
    });
    save(app);
    eprintln!("service chat created: {}", id_pattern(&id));
    Ok((id, orgs))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEmoji {
    path: String,
    name: String,
    /// The organization the user is in now.
    org: Option<u64>,
}

/// Adds a custom emoji: the picked image goes to the organization's service chat with the
/// caption `:name:`. Returns the new sets.
#[tauri::command]
pub async fn add_custom_emoji(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    emoji: NewEmoji,
) -> Result<Vec<EmojiSet>, String> {
    let allowed = state
        .allowed_files
        .lock()
        .map(|a| a.contains(&emoji.path))
        .unwrap_or(false);
    if !allowed {
        return Err(format!("the file was not picked by the user: {}", emoji.path));
    }
    if let Some(e) = name_error(&app, &emoji.name) {
        return Err(e);
    }
    let bytes = tokio::fs::read(&emoji.path)
        .await
        .map_err(|e| format!("{}: {e}", emoji.path))?;
    let prepared = tokio::task::spawn_blocking(move || prepare(bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| {
            match e {
                PrepareError::Unsupported => text(
                    &app,
                    "Нужна картинка PNG, JPEG, WebP или GIF.",
                    "Pick a PNG, JPEG, WebP or GIF image.",
                ),
                PrepareError::BigGif => text(
                    &app,
                    "GIF больше 128 КБ: уменьшите его.",
                    "The GIF is over 128 KB: make it smaller.",
                ),
                PrepareError::Big => text(&app, "Картинка слишком большая.", "The image is too large."),
            }
            .to_owned()
        })?;
    let live = live(&state).await?;
    let _busy = state.service.busy.lock().await;
    let (chat, orgs) = chat_for(&app, &live, emoji.org).await?;
    let taken = sets(&app)
        .iter()
        .any(|s| s.chat_id.0 == chat && s.by_name(&emoji.name).is_some());
    if taken {
        return Err(taken_error(&app));
    }
    let payload = request_id();
    let file_name = format!("{}.{}", emoji.name, prepared.ext);
    let size = prepared.bytes.len();
    let mut web = live.web.clone_light();
    let up = web
        .upload_file(&chat, &payload, &file_name, prepared.mime, prepared.bytes)
        .await
        .map_err(|e| e.to_string())?;
    let file_id = up
        .get("file_id")
        .and_then(Value::as_str)
        .ok_or("Telemost did not return file_id")?;
    let item = json!({ "Image": { "Width": prepared.width, "Height": prepared.height,
        "FileInfo": { "Id2": file_id, "Name": file_name, "Size": size } } });
    let msg = json!({ "Plain": { "ChatId": chat, "PayloadId": payload,
        "Gallery": { "Items": [item], "Text": ce::caption(&emoji.name) } } });
    push(&live, msg).await?;
    refresh(&app, &live, &chat, orgs).await?;
    emit(&app);
    Ok(sets(&app))
}

/// Renames an own emoji: a new caption on its message keeps its reaction type.
#[tauri::command]
pub async fn rename_custom_emoji(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Timestamp,
    name: String,
) -> Result<Vec<EmojiSet>, String> {
    if let Some(e) = name_error(&app, &name) {
        return Err(e);
    }
    let live = live(&state).await?;
    let _busy = state.service.busy.lock().await;
    let taken = sets(&app)
        .iter()
        .any(|s| s.chat_id.0 == chat_id && s.emoji.iter().any(|e| e.name == name && e.ts != ts));
    if taken {
        return Err(taken_error(&app));
    }
    let original = original_plain(&live, &chat_id, ts).await?;
    push(&live, wire::edit_whole(&original, &chat_id, ts, &ce::caption(&name))).await?;
    let orgs = organizations_of(&app, &chat_id);
    refresh(&app, &live, &chat_id, orgs).await?;
    emit(&app);
    Ok(sets(&app))
}

/// Deletes an own emoji: its message goes, and its reactions show as unknown.
#[tauri::command]
pub async fn delete_custom_emoji(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    chat_id: String,
    ts: Timestamp,
) -> Result<Vec<EmojiSet>, String> {
    let live = live(&state).await?;
    let _busy = state.service.busy.lock().await;
    push(&live, wire::delete(&chat_id, ts)).await?;
    let orgs = organizations_of(&app, &chat_id);
    refresh(&app, &live, &chat_id, orgs).await?;
    emit(&app);
    Ok(sets(&app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_images() {
        assert_eq!(sniff(b"\x89PNG\r\n"), "image/png");
        assert_eq!(sniff(b"GIF89a"), "image/gif");
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8"), "image/webp");
        assert_eq!(sniff(b"<html>"), "application/octet-stream");
    }

    #[test]
    fn prepares_still_images_and_gifs() {
        let img = image::RgbaImage::from_pixel(300, 150, image::Rgba([10, 20, 30, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let p = prepare(png.into_inner()).unwrap();
        assert_eq!((p.width, p.height, p.ext), (128, 64, "png"));
        assert!(matches!(
            prepare(b"not an image".to_vec()),
            Err(PrepareError::Unsupported)
        ));
        let mut gif = b"GIF89a".to_vec();
        gif.extend([1, 0, 1, 0, 0, 0, 0]);
        gif.resize(MAX_UPLOAD + 1, 0);
        assert!(matches!(prepare(gif), Err(PrepareError::BigGif)));
    }

    #[test]
    fn finds_marked_chats() {
        let v = json!({"data": {"chats": [
            {"chat_id": "0/0/a", "name": SERVICE_CHAT_NAME, "description": format!("x {SERVICE_MARKER}"), "organization_ids": [7]},
            {"chat_id": "0/0/b", "name": SERVICE_CHAT_NAME, "description": "someone's group"},
            {"chat_id": "0/0/c", "name": "other", "description": SERVICE_MARKER},
        ]}});
        assert_eq!(marked_chats(&v), vec![("0/0/a".to_owned(), vec![7])]);
    }

    #[test]
    fn drops_a_cache_of_the_old_scheme() {
        let old = json!({"me": "u", "looked_ms": 5, "aliased": ["0/0/a"], "sets": [{"set": {"chat_id": "0/0/a",
            "organizations": [7], "emoji": [], "taken": []}, "last_ts": 1, "last_edit_ts": 2}]});
        let d = Disk::load(&serde_json::to_vec(&old).unwrap());
        assert_eq!(
            (d.marker.as_str(), d.sets.len(), d.aliased.len()),
            (SERVICE_MARKER, 0, 0)
        );
        let mut current = old;
        current["marker"] = json!(SERVICE_MARKER);
        let d = Disk::load(&serde_json::to_vec(&current).unwrap());
        assert_eq!((d.me.as_str(), d.sets.len(), d.looked_ms), ("u", 1, 5));
        assert_eq!(Disk::load(b"not json").marker, SERVICE_MARKER);
    }

    #[test]
    fn reads_an_older_disk_cache() {
        let old = json!({"me": "u", "searched_ms": 5, "sets": [{"set": {"chat_id": "0/0/a",
            "organizations": [7], "emoji": [], "taken": []}, "last_ts": 1, "last_edit_ts": 2}]});
        let d: Disk = serde_json::from_value(old).unwrap();
        assert_eq!(
            (d.me.as_str(), d.sets.len(), d.looked_ms, d.aliased.len()),
            ("u", 1, 0, 0)
        );
    }

    #[test]
    fn tells_a_chat_the_user_left() {
        let left = json!({"Chats": [{"ChatId": "0/0/a", "Counters": {}, "Status": 2}],
            "ErrorInfos": [{"Message": "user not in chat members or it's a public chat and user/chat org ids don't match", "WorkerId": 76}],
            "Status": 2});
        assert!(not_member(&left));
        assert!(!not_member(&json!({"Chats": [{"ChatId": "0/0/a", "Messages": []}]})));
    }

    #[test]
    fn checks_the_aliased_chat() {
        let answer = |c: Value| json!({"status": "ok", "data": {"chats": [c]}});
        let found = |invite: Option<&str>, joinable| {
            Some(AliasedChat {
                id: "0/0/a".to_owned(),
                organizations: vec![7],
                invite: invite.map(str::to_owned),
                joinable,
            })
        };
        // What a non-member gets: no members, the right to join and the invite.
        let ours = answer(json!({"chat_id": "0/0/a", "name": SERVICE_CHAT_NAME,
            "description": format!("x {SERVICE_MARKER}"), "organization_ids": [7],
            "invite_hash": "h", "members": [], "rights": ["join", "read"]}));
        assert_eq!(aliased_chat(&ours, 7), found(Some("h"), true));
        assert_eq!(aliased_chat(&ours, 8), None);
        let preview = answer(json!({"chat_id": "0/0/a", "name": SERVICE_CHAT_NAME}));
        assert_eq!(aliased_chat(&preview, 7), found(None, false));
        let member = answer(
            json!({"chat_id": "0/0/a", "name": SERVICE_CHAT_NAME, "organization_ids": [7],
            "invite_hash": "h", "rights": ["read", "write", "change"]}),
        );
        assert_eq!(aliased_chat(&member, 7), found(Some("h"), false));
        let unmarked = answer(json!({"chat_id": "0/0/b", "name": SERVICE_CHAT_NAME, "description": "a group"}));
        assert_eq!(aliased_chat(&unmarked, 7), None);
        let other = answer(json!({"chat_id": "0/0/c", "name": "other", "description": SERVICE_MARKER}));
        assert_eq!(aliased_chat(&other, 7), None);
        assert_eq!(aliased_chat(&json!({"status": "ok", "data": {"chats": []}}), 7), None);
    }
}
