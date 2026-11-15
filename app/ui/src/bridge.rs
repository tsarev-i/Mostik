//! Tauri command calls and event subscriptions. Without Tauri (in a browser) the UI runs on demo data.

pub use mostik_core::event::LiveEvent;
use mostik_core::{ChatId, Message, Timestamp, UnreadBadge, UserId};
pub use mostik_ipc::cmd::ReplyRef;
use mostik_ipc::cmd::{self, Command};
pub use mostik_ipc::{
    Boot, CallInfo, ChatDetails, Dropped, FoundMessages, HistoryPage, OpenTarget, PendingFile, Ring, Sent, Threads,
    UpdateStatus,
};
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    /// `invoke` with options: raw bytes as the arguments and request headers.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke_with(cmd: &str, args: JsValue, options: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn tauri_listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue>;
}

/// Whether the UI runs inside Tauri (otherwise it is the browser demo).
pub fn available() -> bool {
    js_sys::Reflect::has(&js_sys::global(), &JsValue::from_str("__TAURI__")).unwrap_or(false)
}

/// Calls command `C`: its arguments go as its one parameter, [`cmd::ARGS`]. Arguments that
/// serialize to null or undefined (today only `()`, a command without arguments) are left out,
/// so such a command gets `{}`.
async fn call<C: Command>(args: C::Args) -> Result<C::Out, String> {
    let params = js_sys::Object::new();
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| e.to_string())?;
    if !args.is_undefined() && !args.is_null() {
        js_sys::Reflect::set(&params, &cmd::ARGS.into(), &args).map_err(|e| format!("{e:?}"))?;
    }
    invoke(C::NAME, params.into()).await
}

/// The arguments of a command about a whole chat.
fn chat_args(chat_id: &str) -> cmd::ChatArgs {
    cmd::ChatArgs {
        chat_id: ChatId(chat_id.into()),
    }
}

async fn invoke<T: DeserializeOwned>(cmd: &str, args: JsValue) -> Result<T, String> {
    // In the browser demo there is no backend: an error instead of a JS exception that kills the UI.
    if !available() {
        return Err(format!("{cmd}: no Tauri backend"));
    }
    let value = tauri_invoke(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| format!("{cmd}: {e}"))
}

/// Subscribes to a backend event for the whole lifetime of the window.
pub async fn listen<T: DeserializeOwned + 'static>(event: &str, mut handler: impl FnMut(T) + 'static) {
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        let payload = js_sys::Reflect::get(&e, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
        match serde_wasm_bindgen::from_value::<T>(payload) {
            Ok(v) => handler(v),
            Err(err) => leptos::logging::warn!("event not parsed: {err}"),
        }
    });
    if let Err(e) = tauri_listen(event, &closure).await {
        leptos::logging::warn!("listen {event}: {e:?}");
    }
    closure.forget();
}

pub const EVENT: &str = "mostik://event";
pub const SNAPSHOT_EVENT: &str = "mostik://snapshot";
/// The session expired while the app was running: show the login screen with this reason.
pub const SIGNED_OUT_EVENT: &str = "mostik://signed_out";
/// The current call changed: [`CallInfo`] or `null`.
pub const CALL_EVENT: &str = "mostik://call";
/// The call ringing for the user changed: [`Ring`] or `null`.
pub const RING_EVENT: &str = "mostik://ring";

/// A new [`UpdateStatus`]: an update was found or is being installed.
pub const UPDATE_EVENT: &str = "mostik://update";

pub async fn bootstrap() -> Result<Boot, String> {
    call::<cmd::Bootstrap>(()).await
}

pub async fn login() -> Result<Boot, String> {
    call::<cmd::Login>(()).await
}

pub async fn logout() -> Result<(), String> {
    call::<cmd::Logout>(()).await
}

pub async fn update_status() -> Result<UpdateStatus, String> {
    call::<cmd::UpdateStatus>(()).await
}

pub async fn set_auto_update(enabled: bool) -> Result<(), String> {
    call::<cmd::SetAutoUpdate>(cmd::AutoUpdateArgs { enabled }).await
}

/// Checks for a new version now.
pub async fn check_update() -> Result<UpdateStatus, String> {
    call::<cmd::CheckUpdate>(()).await
}

/// Installs the found update and restarts the app; returns only on failure.
pub async fn install_update() -> Result<(), String> {
    call::<cmd::InstallUpdate>(()).await
}

pub async fn history(
    chat_id: &str,
    before: Option<Timestamp>,
    thread: Option<Timestamp>,
) -> Result<HistoryPage, String> {
    let args = cmd::HistoryArgs {
        chat_id: ChatId(chat_id.into()),
        before,
        thread,
    };
    call::<cmd::History>(args).await
}

pub async fn mark_read(chat_id: &str) -> Result<(), String> {
    call::<cmd::MarkRead>(chat_args(chat_id)).await
}

pub async fn followed_threads() -> Result<Threads, String> {
    call::<cmd::FollowedThreads>(()).await
}

pub async fn mark_thread_read(chat_id: &str, parent: Timestamp, ts: Timestamp, seq_no: u64) -> Result<(), String> {
    let args = cmd::ThreadReadArgs {
        chat_id: ChatId(chat_id.into()),
        parent,
        ts,
        seq_no,
    };
    call::<cmd::MarkThreadRead>(args).await
}

pub async fn send_message(
    chat_id: &str,
    text: &str,
    thread: Option<Timestamp>,
    reply: Option<ReplyRef>,
) -> Result<Message, String> {
    let args = cmd::SendArgs {
        chat_id: ChatId(chat_id.into()),
        text: text.into(),
        thread,
        reply,
    };
    call::<cmd::SendMessage>(args).await
}

pub async fn edit_message(m: &Message, text: &str) -> Result<(), String> {
    let args = cmd::EditArgs {
        chat_id: m.chat_id.clone(),
        ts: m.ts,
        text: text.into(),
        thread: m.thread_parent,
        whole: !m.attachments.is_empty() || !m.forwarded.is_empty(),
    };
    call::<cmd::EditMessage>(args).await
}

pub async fn delete_message(m: &Message) -> Result<(), String> {
    let args = cmd::MsgArgs {
        chat_id: m.chat_id.clone(),
        ts: m.ts,
        thread: m.thread_parent,
    };
    call::<cmd::DeleteMessage>(args).await
}

pub async fn set_reaction(m: &Message, kind: u64, add: bool) -> Result<(), String> {
    let args = cmd::ReactionArgs {
        chat_id: m.chat_id.clone(),
        ts: m.ts,
        kind,
        add,
        thread: m.thread_parent,
    };
    call::<cmd::SetReaction>(args).await
}

pub async fn set_important(m: &Message, important: bool) -> Result<(), String> {
    let args = cmd::ImportantArgs {
        chat_id: m.chat_id.clone(),
        ts: m.ts,
        important,
        thread: m.thread_parent,
    };
    call::<cmd::SetImportant>(args).await
}

pub async fn typing(chat_id: &str) -> Result<(), String> {
    call::<cmd::Typing>(chat_args(chat_id)).await
}

/// Downloads an attachment to "Downloads" and shows it in its folder; returns the path.
pub async fn save_file(file_id: &str, name: &str) -> Result<String, String> {
    let args = cmd::FileArgs {
        file_id: file_id.into(),
        name: name.into(),
    };
    call::<cmd::SaveFile>(args).await
}

/// Attachment URL for `<img>`: the backend serves it over the `mostikfile` scheme.
pub fn file_url(file_id: &str, size: Option<&str>) -> String {
    let id = String::from(js_sys::encode_uri_component(file_id));
    let windows = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains("Windows"));
    let base = if windows {
        "http://mostikfile.localhost"
    } else {
        "mostikfile://localhost"
    };
    match size {
        Some(s) => format!("{base}/{id}?size={s}"),
        None => format!("{base}/{id}"),
    }
}

/// Passes to the backend what affects notifications and the tray.
pub async fn set_preferences(s: &crate::settings::Settings) -> Result<(), String> {
    use crate::settings::NotifyMode;
    let notify = match s.notify {
        NotifyMode::All => cmd::NotifyMode::All,
        NotifyMode::DirectAndMentions => cmd::NotifyMode::Direct,
        NotifyMode::Nothing => cmd::NotifyMode::Nothing,
    };
    let args = cmd::PrefsArgs {
        notify,
        preview: s.notify_preview,
        lang: s.lang.code().into(),
        sound: s.sound.clone(),
    };
    call::<cmd::SetPreferences>(args).await
}

/// Plays a notification sound (the backend plays them).
pub async fn play_sound(id: &str) -> Result<(), String> {
    call::<cmd::PlaySound>(cmd::SoundArgs { id: id.into() }).await
}

/// Application window title (document.title does not affect the window in WebKitGTK).
pub async fn set_window_title(title: &str) -> Result<(), String> {
    call::<cmd::SetWindowTitle>(cmd::TitleArgs { title: title.into() }).await
}

/// The unread mark on the tray icon, the taskbar button or the Dock icon.
pub async fn set_unread_badge(badge: UnreadBadge, count: u32) -> Result<(), String> {
    call::<cmd::SetUnreadBadge>(cmd::BadgeArgs { badge, count }).await
}

/// Window zoom: in the app it is the webview zoom, like Ctrl+= in Slack.
pub async fn set_zoom(percent: u32) -> Result<(), String> {
    let args = cmd::ZoomArgs {
        scale: f64::from(percent) / 100.0,
    };
    call::<cmd::SetZoom>(args).await
}

pub async fn user_profile(guid: &str) -> Result<mostik_core::UserProfile, String> {
    call::<cmd::UserProfile>(cmd::GuidArgs {
        guid: UserId(guid.into()),
    })
    .await
}

pub async fn open_dm(guid: &str) -> Result<mostik_core::Chat, String> {
    call::<cmd::OpenDm>(cmd::GuidArgs {
        guid: UserId(guid.into()),
    })
    .await
}

pub async fn search_people(query: &str, chat_id: Option<&str>) -> Result<Vec<mostik_core::UserProfile>, String> {
    let args = cmd::PeopleSearchArgs {
        query: query.into(),
        chat_id: chat_id.map(|c| ChatId(c.into())),
    };
    call::<cmd::SearchPeople>(args).await
}

pub async fn create_channel(
    name: &str,
    description: &str,
    members: Vec<String>,
    public: bool,
) -> Result<mostik_core::Chat, String> {
    let args = cmd::CreateChannelArgs {
        name: name.into(),
        description: description.into(),
        members: members.into_iter().map(UserId).collect(),
        public,
    };
    call::<cmd::CreateChannel>(args).await
}

pub async fn chat_members(chat_id: &str) -> Result<Vec<mostik_core::ChatMember>, String> {
    call::<cmd::ChatMembers>(chat_args(chat_id)).await
}

pub async fn change_members(chat_id: &str, guids: Vec<String>, add: bool) -> Result<(), String> {
    let args = cmd::MembersArgs {
        chat_id: ChatId(chat_id.into()),
        guids: guids.into_iter().map(UserId).collect(),
        add,
    };
    call::<cmd::ChangeMembers>(args).await
}

pub async fn update_chat(chat_id: &str, name: &str, description: &str) -> Result<ChatDetails, String> {
    let args = cmd::UpdateChatArgs {
        chat_id: ChatId(chat_id.into()),
        name: name.into(),
        description: description.into(),
    };
    call::<cmd::UpdateChat>(args).await
}

pub async fn leave_chat(chat_id: &str) -> Result<(), String> {
    call::<cmd::LeaveChat>(chat_args(chat_id)).await
}

pub async fn chat_details(chat_id: &str) -> Result<ChatDetails, String> {
    call::<cmd::ChatDetails>(chat_args(chat_id)).await
}

pub const FILES_EVENT: &str = "mostik://files";
/// Files are dragged over the window: where, in logical pixels of the window; `None` when the drag ends.
pub const DRAG_EVENT: &str = "mostik://drag";

/// The custom emoji sets changed.
pub const EMOJI_EVENT: &str = "mostik://emoji";
/// The backend showed a notification: play the sound.
/// The user clicked a message notification: open that message.
pub const OPEN_EVENT: &str = "mostik://open";

/// A file taken off the message: the app no longer keeps a pasted one.
pub async fn forget_file(path: &str) -> Result<(), String> {
    call::<cmd::ForgetFile>(cmd::PathArgs { path: path.into() }).await
}

pub async fn pick_files() -> Result<Vec<PendingFile>, String> {
    call::<cmd::PickFiles>(()).await
}

/// Hands the contents of a pasted or dropped file to the app, which keeps it to be sent like a
/// picked one. The bytes go as they are, not as JSON: the one command that does not go through
/// [`call`] with an argument struct, since a large file would not fit JSON well.
pub async fn keep_file(name: &str, data: &js_sys::ArrayBuffer) -> Result<PendingFile, String> {
    if !available() {
        return Err(format!("{}: no Tauri backend", cmd::KeepFile::NAME));
    }
    let headers = js_sys::Object::new();
    let name = js_sys::encode_uri_component(name);
    js_sys::Reflect::set(&headers, &"x-file-name".into(), &name).map_err(|e| format!("{e:?}"))?;
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"headers".into(), &headers).map_err(|e| format!("{e:?}"))?;
    let body = js_sys::Uint8Array::new(data);
    let value = tauri_invoke_with(cmd::KeepFile::NAME, body.into(), options.into())
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| format!("{}: {e}", cmd::KeepFile::NAME))
}

pub async fn send_files(
    chat_id: &str,
    thread: Option<Timestamp>,
    paths: Vec<String>,
    text: &str,
) -> Result<Vec<Message>, String> {
    let args = cmd::SendFilesArgs {
        chat_id: ChatId(chat_id.into()),
        thread,
        paths,
        text: text.into(),
    };
    call::<cmd::SendFiles>(args).await
}

pub async fn presence(guids: Vec<String>) -> Result<Vec<mostik_core::PresenceInfo>, String> {
    let args = cmd::GuidsArgs {
        guids: guids.into_iter().map(UserId).collect(),
    };
    call::<cmd::Presence>(args).await
}

pub async fn set_status(availability: u64, emoji: &str, text: &str, duration: Option<u64>) -> Result<(), String> {
    let args = cmd::StatusArgs {
        availability,
        emoji: emoji.into(),
        text: text.into(),
        duration,
    };
    call::<cmd::SetStatus>(args).await
}

pub async fn test_notification() -> Result<(), String> {
    call::<cmd::TestNotification>(()).await
}

pub async fn open_yandex_id() -> Result<(), String> {
    call::<cmd::OpenYandexId>(()).await
}

pub async fn open_url(url: &str) -> Result<(), String> {
    call::<cmd::OpenUrl>(cmd::UrlArgs { url: url.into() }).await
}

/// Server-side message search in the listed chats.
pub async fn search_messages(query: &str, chat_ids: Vec<String>) -> Result<FoundMessages, String> {
    let args = cmd::MessageSearchArgs {
        query: query.into(),
        chat_ids: chat_ids.into_iter().map(ChatId).collect(),
    };
    call::<cmd::SearchMessages>(args).await
}

pub async fn set_notify_level(chat_id: &str, level: mostik_core::NotifyLevel) -> Result<(), String> {
    let args = cmd::NotifyLevelArgs {
        chat_id: ChatId(chat_id.into()),
        level,
    };
    call::<cmd::SetNotifyLevel>(args).await
}

/// Writes the organization's chat folders; returns those of every organization.
pub async fn save_chat_folders(
    org: &str,
    folders: &[mostik_core::ChatFolder],
) -> Result<Vec<mostik_core::ChatFolder>, String> {
    let args = cmd::FoldersArgs {
        org: org.into(),
        folders: folders.to_vec(),
    };
    call::<cmd::SaveChatFolders>(args).await
}

/// Returns the new list of starred chats.
pub async fn set_starred(chat_id: &str, starred: bool) -> Result<Vec<ChatId>, String> {
    let args = cmd::StarArgs {
        chat_id: ChatId(chat_id.into()),
        starred,
    };
    call::<cmd::SetStarred>(args).await
}

pub async fn mark_unread(chat_id: &str, ts: Timestamp, seq_no: u64) -> Result<(), String> {
    let args = cmd::UnreadArgs {
        chat_id: ChatId(chat_id.into()),
        ts,
        seq_no,
    };
    call::<cmd::MarkUnread>(args).await
}

pub async fn pin_message(chat_id: &str, ts: Option<Timestamp>) -> Result<(), String> {
    let args = cmd::PinArgs {
        chat_id: ChatId(chat_id.into()),
        ts,
    };
    call::<cmd::PinMessage>(args).await
}

pub async fn forward_message(m: &Message, to_chat: &str, text: &str) -> Result<Sent, String> {
    let args = cmd::ForwardArgs {
        from_chat: m.chat_id.clone(),
        ts: m.ts,
        thread: m.thread_parent,
        to_chat: ChatId(to_chat.into()),
        text: text.into(),
    };
    call::<cmd::ForwardMessage>(args).await
}

/// `url` joins a call already going on in the chat instead of starting one.
pub async fn start_call(chat_id: &str, label: &str, url: Option<&str>) -> Result<(), String> {
    let args = cmd::StartCallArgs {
        chat_id: ChatId(chat_id.into()),
        label: label.into(),
        url: url.map(Into::into),
    };
    call::<cmd::StartCall>(args).await
}

pub async fn focus_call() -> Result<(), String> {
    call::<cmd::FocusCall>(()).await
}

pub async fn leave_call() -> Result<(), String> {
    call::<cmd::LeaveCall>(()).await
}

pub async fn ring_info() -> Result<Option<Ring>, String> {
    call::<cmd::RingInfo>(()).await
}

/// "Accept" or "Decline" in the incoming call dialog.
pub async fn answer_ring(accept: bool) -> Result<(), String> {
    call::<cmd::AnswerRing>(cmd::AnswerArgs { accept }).await
}

pub async fn current_call() -> Result<Option<CallInfo>, String> {
    call::<cmd::CurrentCall>(()).await
}

pub async fn recommended_chats() -> Result<Vec<mostik_core::Chat>, String> {
    call::<cmd::RecommendedChats>(()).await
}

pub async fn join_chat(chat_id: &str) -> Result<mostik_core::Chat, String> {
    call::<cmd::JoinChat>(chat_args(chat_id)).await
}

/// Custom emoji sets of the user's organizations (from the disk cache until the chats are read).
pub async fn custom_emoji() -> Result<Vec<mostik_core::custom_emoji::EmojiSet>, String> {
    call::<cmd::CustomEmoji>(()).await
}

/// Picks an image for a new custom emoji.
pub async fn pick_emoji_image() -> Result<Option<PendingFile>, String> {
    call::<cmd::PickEmojiImage>(()).await
}

/// Adds a custom emoji to the service chat of organization `org`; returns the new sets.
pub async fn add_custom_emoji(
    path: &str,
    name: &str,
    org: Option<u64>,
) -> Result<Vec<mostik_core::custom_emoji::EmojiSet>, String> {
    let args = cmd::NewEmoji {
        path: path.into(),
        name: name.into(),
        org,
    };
    call::<cmd::AddCustomEmoji>(args).await
}

/// Deletes an own custom emoji; returns the new sets.
pub async fn delete_custom_emoji(
    chat_id: &str,
    ts: Timestamp,
) -> Result<Vec<mostik_core::custom_emoji::EmojiSet>, String> {
    let args = cmd::DeleteEmojiArgs {
        chat_id: ChatId(chat_id.into()),
        ts,
    };
    call::<cmd::DeleteCustomEmoji>(args).await
}

/// A file the user has just picked, for a preview: the backend serves only picked files.
pub fn local_file_url(path: &str) -> String {
    file_url(&format!("local/{path}"), None)
}

/// Image of a custom emoji: the backend serves it from its disk cache.
pub fn emoji_url(file_id: &str) -> String {
    file_url(&format!("emoji/{file_id}"), None)
}

#[cfg(test)]
mod tests {
    use mostik_ipc::cmd;

    /// Public functions here that are not the wrapper of a JSON command, and why.
    const NOT_WRAPPERS: [(&str, &str); 2] = [
        ("listen", "subscribes to an event, calls no command"),
        (
            "keep_file",
            "sends raw bytes with cmd::KeepFile::NAME, not call::<cmd::KeepFile>",
        ),
    ];

    fn snake(ty: &str) -> String {
        let mut out = String::new();
        for (i, ch) in ty.chars().enumerate() {
            if ch.is_ascii_uppercase() && i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        }
        out
    }

    /// Each wrapper calls the command it is named after, exactly once: `typing` cannot call
    /// `call::<cmd::LeaveChat>`, which has the same arguments and result and so compiles. And
    /// every command has exactly one wrapper.
    #[test]
    fn wrappers_call_the_command_they_are_named_after() {
        // The checkout may have CRLF line ends (`core.autocrlf`).
        let src = include_str!("bridge.rs").replace("\r\n", "\n");
        let src = src.as_str();
        // Built from parts, so this test's own text is not taken for a function.
        let marker = concat!("pub async ", "fn ");
        let mut wrapped = Vec::new();
        for (at, _) in src.match_indices(marker) {
            let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
            if line_start != at {
                continue;
            }
            let rest = &src[at + marker.len()..];
            let name_end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).expect("name");
            let name = &rest[..name_end];
            let body_start = rest.find("{\n").expect("body");
            let body_end = body_start + rest[body_start..].find("\n}\n").expect("end of the body");
            let body: String = rest[body_start..body_end]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let calls: Vec<&str> = body
                .match_indices("call::<cmd::")
                .map(|(i, m)| {
                    let ty = &body[i + m.len()..];
                    &ty[..ty.find('>').expect("end of the command type")]
                })
                .collect();
            if let Some((_, why)) = NOT_WRAPPERS.iter().find(|(n, _)| *n == name) {
                assert!(calls.is_empty(), "{name} ({why}) calls {calls:?}");
                if name == "keep_file" {
                    // The wrapper of cmd::KeepFile, by its raw transport.
                    assert!(
                        body.contains("cmd::KeepFile::NAME"),
                        "keep_file does not use cmd::KeepFile::NAME"
                    );
                    wrapped.push(name.to_owned());
                }
                continue;
            }
            assert_eq!(calls.len(), 1, "{name}: calls {calls:?}, not exactly one command");
            assert_eq!(snake(calls[0]), name, "{name} calls cmd::{}", calls[0]);
            wrapped.push(name.to_owned());
        }
        wrapped.sort_unstable();
        let mut all: Vec<String> = cmd::ALL.iter().map(|c| c.name.to_owned()).collect();
        all.sort_unstable();
        assert_eq!(wrapped, all, "each command needs exactly one wrapper");
    }

    /// Commands are called only from their wrappers: outside the tests the file has one
    /// `call::<cmd::…>` per JSON command (all but `keep_file`) and calls `invoke` only from
    /// `call`, so a private helper cannot call another command on the side.
    #[test]
    fn commands_are_called_only_from_wrappers() {
        let src = include_str!("bridge.rs").replace("\r\n", "\n");
        let code = &src[..src.find(concat!("#[cfg(", "test)]")).expect("test module")];
        assert_eq!(code.matches("call::<cmd::").count(), cmd::ALL.len() - 1);
        let invokes = code
            .match_indices("invoke(")
            .filter(|&(i, _)| !code[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
            .count();
        assert_eq!(invokes, 1, "invoke( outside call");
    }
}
