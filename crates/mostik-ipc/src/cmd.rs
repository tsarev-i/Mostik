//! Tauri commands: their names, arguments and results.
//!
//! Every command that takes arguments takes one parameter, [`ARGS`], holding its argument struct:
//! the UI sends `{ "args": { … } }` and the backend reads `args: Args<cmd::SendMessage>`. Each
//! command is a type implementing [`Command`], which ties its name, arguments and result
//! together: the backend's function takes [`Args`] and returns [`Out`] of it, and the UI calls
//! `call::<cmd::SendMessage>(args)`, so neither side can use another struct or result. Tauri
//! knows a command by the backend's function name, which the compiler cannot check: the backend
//! tests that its functions are exactly [`ALL`], each named after the type whose `Args` and `Out`
//! it uses.
//!
//! To add a command:
//! 1. a line in the `commands!` list below, with its argument struct (`()` for none) and result;
//! 2. a `#[tauri::command]` function named as the command, with `args: Args<cmd::X>` (if it has
//!    arguments) and `-> Result<Out<cmd::X>, String>` (or `-> Out<cmd::X>`);
//! 3. the function in `generate_handler!` in `app/src-tauri/src/main.rs`;
//! 4. a wrapper in `app/ui/src/bridge.rs` calling `call::<cmd::X>(args)`.
//!
//! Always write a command as `cmd::X`, never import it: `ChatDetails` and `UpdateStatus` are both
//! a command here and a result type of the crate, and `cmd::` keeps the two apart everywhere.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use mostik_core::custom_emoji::EmojiSet;
use mostik_core::{
    Chat, ChatFolder, ChatId, ChatMember, Message, NotifyLevel, PresenceInfo, Timestamp, UnreadBadge, UserId,
};

use crate::{Boot, CallInfo, FoundMessages, HistoryPage, PendingFile, Ring, Sent, Threads};

/// The name of the one parameter of every command that takes arguments.
pub const ARGS: &str = "args";

// History and threads.

/// A history page of a chat or thread: the latest messages before `before` (exclusive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryArgs {
    pub chat_id: ChatId,
    pub before: Option<Timestamp>,
    /// The parent of the thread, for a thread page.
    pub thread: Option<Timestamp>,
    /// How many records at most; `None` for the page size the app picks. One message is looked
    /// for with `before` one past its time and a limit of 1.
    pub limit: Option<u32>,
}

/// Just a chat: for `mark_read`, `typing` and the other commands about a whole chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatArgs {
    pub chat_id: ChatId,
}

/// The thread of message `parent` is read up to its reply `ts` with number `seq_no` (the thread
/// chat's own numbering).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReadArgs {
    pub chat_id: ChatId,
    pub parent: Timestamp,
    pub ts: Timestamp,
    pub seq_no: u64,
}

// Messages.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendArgs {
    pub chat_id: ChatId,
    pub text: String,
    pub thread: Option<Timestamp>,
    pub reply: Option<ReplyRef>,
    /// How the message goes: without a sound, as important.
    #[serde(default)]
    pub options: SendOptions,
}

/// How a new message goes, as the buttons by the official client's field set it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOptions {
    /// The members get it without a sound or a notification.
    #[serde(default)]
    pub silent: bool,
    /// It goes marked as important, as "Mark as important" marks a sent one.
    #[serde(default)]
    pub important: bool,
    /// It goes on from a message just sent (files after the text of a reply): none of its
    /// messages is the first, which alone carries the two above.
    #[serde(default)]
    pub follows: bool,
}

/// What a sent message replies to: message `ts` of the chat (or thread) it is sent to, with the
/// quoted fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyRef {
    pub ts: Timestamp,
    pub quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditArgs {
    pub chat_id: ChatId,
    pub ts: Timestamp,
    pub text: String,
    pub thread: Option<Timestamp>,
    /// The message has more than text (images, files, forwarded messages): the backend resends
    /// all of it.
    pub whole: bool,
}

/// Just a message: for `delete_message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsgArgs {
    pub chat_id: ChatId,
    pub ts: Timestamp,
    pub thread: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionArgs {
    pub chat_id: ChatId,
    pub ts: Timestamp,
    pub kind: u64,
    pub add: bool,
    pub thread: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportantArgs {
    pub chat_id: ChatId,
    pub ts: Timestamp,
    pub important: bool,
    pub thread: Option<Timestamp>,
}

/// The chat becomes unread from message `ts` with number `seq_no`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadArgs {
    pub chat_id: ChatId,
    pub ts: Timestamp,
    pub seq_no: u64,
}

/// Pins message `ts` in the chat, or unpins with `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinArgs {
    pub chat_id: ChatId,
    pub ts: Option<Timestamp>,
}

/// Forwards message `ts` of `from_chat` (a thread reply: of the thread `thread`) to `to_chat`
/// with a comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardArgs {
    pub from_chat: ChatId,
    pub ts: Timestamp,
    pub thread: Option<Timestamp>,
    pub to_chat: ChatId,
    pub text: String,
}

// Files.

/// Downloads attachment `file_id` to "Downloads" as `name` and shows it in its folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileArgs {
    pub file_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlArgs {
    pub url: String,
}

/// Plays voice message `file_id` (`Voice.FileInfo.Id2`) from `from_ms` milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceArgs {
    pub file_id: String,
    pub from_ms: u64,
}

/// A file taken off the message, as [`crate::PendingFile::path`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathArgs {
    pub path: String,
}

/// Sends picked files (their [`crate::PendingFile::path`]s) with an optional text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendFilesArgs {
    pub chat_id: ChatId,
    pub thread: Option<Timestamp>,
    pub paths: Vec<String>,
    pub text: String,
    /// The UI's id of this sending: its progress comes as [`crate::UploadProgress`] with it, and
    /// [`CancelUpload`] names it.
    pub upload: Option<String>,
    /// How the messages go: without a sound, as important.
    #[serde(default)]
    pub options: SendOptions,
}

/// How [`SendFiles`] fails when an upload stops moving, after `<file name>: `. The UI shows it
/// in its own words.
pub const UPLOAD_STALLED: &str = "the upload stopped, check the connection";

/// The largest file the Messenger storage takes: a larger one is refused with `HTTP 400 bad_file`
/// after the whole of it has gone, so it goes through the user's Yandex Disk, as in the web client.
pub const MAX_UPLOAD: u64 = 50 * 1024 * 1024;

/// How [`SendFiles`] fails, after `<file name>: `, when Yandex Disk has no room for a file over
/// [`MAX_UPLOAD`]. The UI shows it in its own words.
pub const DISK_FULL: &str = "there is no room left on Yandex Disk";

/// How [`SendFiles`] fails, after `<file name>: `, when a file is larger than Yandex Disk takes.
/// The UI shows it in its own words.
pub const DISK_FILE_TOO_LARGE: &str = "the file is larger than Yandex Disk takes";

/// How sending or editing a message fails when Telemost refuses its text as too big
/// (`message too big`). The UI shows it in its own words.
pub const MESSAGE_TOO_LONG: &str = "the message is too long";

/// One file of a sending going on: [`CancelUpload`] stops its upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadFileArgs {
    pub upload: String,
    pub path: String,
}

// People and chats.

/// Just a person: for `user_profile` and `open_dm`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidArgs {
    pub guid: UserId,
}

/// People search for invitations; without `chat_id` relative to the notes chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeopleSearchArgs {
    pub query: String,
    pub chat_id: Option<ChatId>,
}

/// Channels and people of organization `org` (the first one of the user's without it) whose
/// name has `query`, for "Find a conversation".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectorySearchArgs {
    pub query: String,
    pub org: Option<u64>,
}

/// Server-side message search in the listed chats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSearchArgs {
    pub query: String,
    pub chat_ids: Vec<ChatId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelArgs {
    pub name: String,
    pub description: String,
    pub members: Vec<UserId>,
    pub public: bool,
}

/// Adds people to the chat or, with `add` false, removes them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembersArgs {
    pub chat_id: ChatId,
    pub guids: Vec<UserId>,
    pub add: bool,
}

/// A new name and description of the chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChatArgs {
    pub chat_id: ChatId,
    pub name: String,
    pub description: String,
}

// The chat list: notifications, stars, folders.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyLevelArgs {
    pub chat_id: ChatId,
    pub level: NotifyLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StarArgs {
    pub chat_id: ChatId,
    pub starred: bool,
}

/// The chat folders of organization `org`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldersArgs {
    pub org: String,
    pub folders: Vec<ChatFolder>,
}

// Presence.

/// Presence of these people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidsArgs {
    pub guids: Vec<UserId>,
}

/// The user's availability (the protocol code, see [`mostik_core::Availability::code`]) and custom
/// status, for `duration` seconds or until changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusArgs {
    pub availability: u64,
    pub emoji: String,
    pub text: String,
    pub duration: Option<u64>,
}

// Calls.

/// Opens the call of the chat; `url` joins a call already going on there instead of starting one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCallArgs {
    pub chat_id: ChatId,
    /// `#dev` or a person's name, for the call bar.
    pub label: String,
    pub url: Option<String>,
}

/// "Accept" or "Decline" in the incoming call dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerArgs {
    pub accept: bool,
}

// Preferences, notifications and the window.

/// What the backend needs from "Preferences": notifications, previews, language and sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefsArgs {
    pub notify: NotifyMode,
    /// Notifications show the message text.
    pub preview: bool,
    /// `ru` or `en`.
    pub lang: String,
    /// The notification sound.
    pub sound: String,
    /// The tray icon on Linux and Windows; macOS always shows the white one as a template.
    pub tray_icon: TrayIcon,
    /// Check spelling in the message fields (#50): on Linux WebKitGTK does it only when asked.
    pub spell_check: bool,
}

/// The tray icon from "Preferences".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrayIcon {
    /// The white truss on transparency, for a dark panel.
    White,
    /// The app icon, the white truss on a near-black tile.
    App,
}

/// What notifies, from "Preferences".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyMode {
    #[default]
    All,
    /// Direct chats and mentions.
    Direct,
    Nothing,
}

/// Plays notification sound `id`: the preview in the preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundArgs {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleArgs {
    pub title: String,
}

/// The unread mark on the tray icon, the taskbar button or the Dock icon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BadgeArgs {
    pub badge: UnreadBadge,
    pub count: u32,
}

/// Window zoom, 1.0 is 100 %.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomArgs {
    pub scale: f64,
}

// Custom emoji.

/// A new custom emoji: a picked image ([`crate::PendingFile::path`]) and its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEmoji {
    pub path: String,
    pub name: String,
    /// The organization the user is in now.
    pub org: Option<u64>,
}

/// An own custom emoji: its message in the service chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEmojiArgs {
    pub chat_id: ChatId,
    pub ts: Timestamp,
}

// Updates.

/// Background update checks on or off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoUpdateArgs {
    pub enabled: bool,
}

/// The arguments of a command that does not take JSON (`keep_file`, whose body is raw bytes): no
/// value has this type, so `call::<cmd::KeepFile>(…)` cannot be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawBody {}

/// The arguments of command `C`, for the backend's `args: Args<cmd::X>`.
pub type Args<C> = <C as Command>::Args;
/// The result of command `C`, for the backend's `-> Result<Out<cmd::X>, String>`.
pub type Out<C> = <C as Command>::Out;

/// A Tauri command: its name, what it takes and what it returns. One type per command, so the
/// UI's `call::<cmd::EditMessage>(args)` only compiles with the right arguments and result type.
pub trait Command {
    /// The name Tauri knows the command by: the backend's function name.
    const NAME: &'static str;
    /// Sent as the one parameter [`ARGS`]; `()` for a command without arguments, which gets `{}`.
    type Args: Serialize + DeserializeOwned;
    /// The result, `()` for none.
    type Out: Serialize + DeserializeOwned;
}

/// A command as text, for checks that cannot use the types: the backend's test that its
/// `#[tauri::command]` functions match, and the name check below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandInfo {
    pub name: &'static str,
    /// The command's type, e.g. `EditMessage`.
    pub ty: &'static str,
    /// The arguments as written: `()` for none, `RawBody` for `keep_file`.
    pub args: &'static str,
}

macro_rules! commands {
    ($($(#[$attr:meta])* $ty:ident = $name:literal, $args:ty => $out:ty;)*) => {
        $(
            $(#[$attr])*
            #[derive(Debug, Clone, Copy)]
            pub struct $ty;

            impl Command for $ty {
                const NAME: &'static str = $name;
                type Args = $args;
                type Out = $out;
            }
        )*

        /// Every command, for the backend's checks that it registers exactly these and that their
        /// signatures match.
        pub const ALL: &[CommandInfo] = &[$(CommandInfo {
            name: $name,
            ty: stringify!($ty),
            args: stringify!($args),
        },)*];
    };
}

commands! {
    // Session.
    Bootstrap = "bootstrap", () => Boot;
    Login = "login", () => Boot;
    Logout = "logout", () => ();

    // History and threads.
    History = "history", HistoryArgs => HistoryPage;
    MarkRead = "mark_read", ChatArgs => ();
    Typing = "typing", ChatArgs => ();
    FollowedThreads = "followed_threads", () => Threads;
    MarkThreadRead = "mark_thread_read", ThreadReadArgs => ();

    // Messages.
    SendMessage = "send_message", SendArgs => crate::SentText;
    EditMessage = "edit_message", EditArgs => ();
    DeleteMessage = "delete_message", MsgArgs => ();
    SetReaction = "set_reaction", ReactionArgs => ();
    SetImportant = "set_important", ImportantArgs => ();
    MarkUnread = "mark_unread", UnreadArgs => ();
    PinMessage = "pin_message", PinArgs => ();
    ForwardMessage = "forward_message", ForwardArgs => Sent;

    // Files.
    SaveFile = "save_file", FileArgs => String;
    OpenYandexId = "open_yandex_id", () => ();
    OpenUrl = "open_url", UrlArgs => ();
    PickFiles = "pick_files", () => Vec<PendingFile>;
    /// Keeps a pasted or dropped file to be sent like a picked one. Not called with JSON: its body
    /// is the raw bytes of the file and the name goes in the `x-file-name` header (URI-encoded),
    /// so its arguments are [`RawBody`], which no value has.
    KeepFile = "keep_file", RawBody => PendingFile;
    ForgetFile = "forget_file", PathArgs => ();
    SendFiles = "send_files", SendFilesArgs => Vec<Message>;
    /// Stops the upload of one file of a sending: the rest goes without it.
    CancelUpload = "cancel_upload", UploadFileArgs => ();
    /// Plays a voice message, stopping the one playing; returns its length in milliseconds.
    PlayVoice = "play_voice", VoiceArgs => u64;
    /// Stops the voice message playing; returns where it stopped, in milliseconds.
    StopVoice = "stop_voice", () => Option<u64>;

    // People and chats.
    UserProfile = "user_profile", GuidArgs => mostik_core::UserProfile;
    OpenDm = "open_dm", GuidArgs => crate::DirectChat;
    SearchPeople = "search_people", PeopleSearchArgs => Vec<mostik_core::UserProfile>;
    SearchMessages = "search_messages", MessageSearchArgs => FoundMessages;
    SearchDirectory = "search_directory", DirectorySearchArgs => crate::DirectoryHits;
    CreateChannel = "create_channel", CreateChannelArgs => Chat;
    ChatDetails = "chat_details", ChatArgs => crate::ChatDetails;
    ChatMembers = "chat_members", ChatArgs => Vec<ChatMember>;
    LeaveChat = "leave_chat", ChatArgs => ();
    JoinChat = "join_chat", ChatArgs => Chat;
    RecommendedChats = "recommended_chats", () => Vec<Chat>;
    ChangeMembers = "change_members", MembersArgs => ();
    UpdateChat = "update_chat", UpdateChatArgs => crate::ChatDetails;

    // The chat list.
    SetNotifyLevel = "set_notify_level", NotifyLevelArgs => ();
    /// Returns the starred chats after the change.
    SetStarred = "set_starred", StarArgs => Vec<ChatId>;
    /// Returns the folders of every organization after the change.
    SaveChatFolders = "save_chat_folders", FoldersArgs => Vec<ChatFolder>;

    // Presence.
    Presence = "presence", GuidsArgs => Vec<PresenceInfo>;
    SetStatus = "set_status", StatusArgs => ();

    // Calls.
    StartCall = "start_call", StartCallArgs => ();
    FocusCall = "focus_call", () => ();
    LeaveCall = "leave_call", () => ();
    CurrentCall = "current_call", () => Option<CallInfo>;
    RingInfo = "ring_info", () => Option<Ring>;
    AnswerRing = "answer_ring", AnswerArgs => ();

    // Preferences, notifications and the window.
    SetPreferences = "set_preferences", PrefsArgs => ();
    TestNotification = "test_notification", () => ();
    PlaySound = "play_sound", SoundArgs => ();
    SetWindowTitle = "set_window_title", TitleArgs => ();
    SetUnreadBadge = "set_unread_badge", BadgeArgs => ();
    SetZoom = "set_zoom", ZoomArgs => ();

    // Custom emoji.
    CustomEmoji = "custom_emoji", () => Vec<EmojiSet>;
    PickEmojiImage = "pick_emoji_image", () => Option<PendingFile>;
    AddCustomEmoji = "add_custom_emoji", NewEmoji => Vec<EmojiSet>;
    DeleteCustomEmoji = "delete_custom_emoji", DeleteEmojiArgs => Vec<EmojiSet>;

    // Updates.
    UpdateStatus = "update_status", () => crate::UpdateStatus;
    CheckUpdate = "check_update", () => crate::UpdateStatus;
    InstallUpdate = "install_update", () => ();
    SetAutoUpdate = "set_auto_update", AutoUpdateArgs => ();
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = ALL.iter().map(|c| c.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL.len());
    }

    #[test]
    fn args_keep_their_field_names() {
        let args = SendArgs {
            chat_id: ChatId("c1".into()),
            text: "hi".into(),
            thread: None,
            reply: Some(ReplyRef {
                ts: 5,
                quote: Some("q".into()),
            }),
            options: SendOptions {
                silent: true,
                important: false,
                follows: false,
            },
        };
        let expected = json!({
            "chatId": "c1", "text": "hi", "thread": null, "reply": { "ts": 5, "quote": "q" },
            "options": { "silent": true, "important": false, "follows": false },
        });
        assert_eq!(serde_json::to_value(&args).unwrap(), expected);
        assert_eq!(serde_json::from_value::<SendArgs>(expected).unwrap(), args);
        let args = PrefsArgs {
            notify: NotifyMode::Direct,
            preview: true,
            lang: "en".into(),
            sound: "bell".into(),
            tray_icon: TrayIcon::App,
            spell_check: true,
        };
        pinned(
            args,
            json!({ "notify": "direct", "preview": true, "lang": "en", "sound": "bell", "trayIcon": "app", "spellCheck": true }),
        );
        let args = ThreadReadArgs {
            chat_id: ChatId("c1".into()),
            parent: 1,
            ts: 2,
            seq_no: 3,
        };
        pinned(args, json!({ "chatId": "c1", "parent": 1, "ts": 2, "seqNo": 3 }));
    }

    /// Serializes `value`, checks the JSON against `expected` and reads it back unchanged.
    fn pinned<T>(value: T, expected: serde_json::Value)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        assert_eq!(serde_json::to_value(&value).unwrap(), expected);
        assert_eq!(serde_json::from_value::<T>(expected).unwrap(), value);
    }

    /// The UI leaves out a `None` field (`serde_wasm_bindgen` turns it into `undefined`, so the key
    /// is absent, not `null`): a missing optional key must read as `None`.
    #[test]
    fn missing_optional_keys_are_none() {
        let send: SendArgs = serde_json::from_value(json!({ "chatId": "c1", "text": "hi" })).unwrap();
        assert_eq!(
            send,
            SendArgs {
                chat_id: ChatId("c1".into()),
                text: "hi".into(),
                thread: None,
                reply: None,
                options: SendOptions::default(),
            }
        );
        let reply: ReplyRef = serde_json::from_value(json!({ "ts": 5 })).unwrap();
        assert_eq!(reply, ReplyRef { ts: 5, quote: None });
        let history: HistoryArgs = serde_json::from_value(json!({ "chatId": "c1" })).unwrap();
        assert_eq!(
            history,
            HistoryArgs {
                chat_id: ChatId("c1".into()),
                before: None,
                thread: None,
                limit: None,
            }
        );
        let edit: EditArgs =
            serde_json::from_value(json!({ "chatId": "c1", "ts": 10, "text": "x", "whole": false })).unwrap();
        assert_eq!(edit.thread, None);
        let call: StartCallArgs = serde_json::from_value(json!({ "chatId": "c1", "label": "#dev" })).unwrap();
        assert_eq!(call.url, None);
    }

    #[test]
    fn more_args_are_pinned() {
        pinned(
            ForwardArgs {
                from_chat: ChatId("c1".into()),
                ts: 10,
                thread: Some(5),
                to_chat: ChatId("c2".into()),
                text: "look".into(),
            },
            json!({ "fromChat": "c1", "ts": 10, "thread": 5, "toChat": "c2", "text": "look" }),
        );
        pinned(
            FileArgs {
                file_id: "f1".into(),
                name: "a.txt".into(),
            },
            json!({ "fileId": "f1", "name": "a.txt" }),
        );
        pinned(
            MessageSearchArgs {
                query: "hi".into(),
                chat_ids: vec![ChatId("c1".into()), ChatId("c2".into())],
            },
            json!({ "query": "hi", "chatIds": ["c1", "c2"] }),
        );
        pinned(
            StartCallArgs {
                chat_id: ChatId("c1".into()),
                label: "#dev".into(),
                url: None,
            },
            json!({ "chatId": "c1", "label": "#dev", "url": null }),
        );
        pinned(
            NewEmoji {
                path: "kept:1/a.png".into(),
                name: "party".into(),
                org: Some(7),
            },
            json!({ "path": "kept:1/a.png", "name": "party", "org": 7 }),
        );
        pinned(
            BadgeArgs {
                badge: UnreadBadge::Mention,
                count: 3,
            },
            json!({ "badge": "mention", "count": 3 }),
        );
    }

    fn chat() -> ChatId {
        ChatId("c1".into())
    }

    fn guid() -> UserId {
        UserId("u1".into())
    }

    /// The arguments of the history and message commands, key by key: the format changes only
    /// on purpose, and each struct reads back what it wrote.
    #[test]
    fn history_and_message_args_are_pinned() {
        pinned(
            HistoryArgs {
                chat_id: chat(),
                before: Some(10),
                thread: None,
                limit: Some(1),
            },
            json!({ "chatId": "c1", "before": 10, "thread": null, "limit": 1 }),
        );
        pinned(ChatArgs { chat_id: chat() }, json!({ "chatId": "c1" }));
        pinned(
            EditArgs {
                chat_id: chat(),
                ts: 10,
                text: "x".into(),
                thread: Some(5),
                whole: true,
            },
            json!({ "chatId": "c1", "ts": 10, "text": "x", "thread": 5, "whole": true }),
        );
        pinned(
            MsgArgs {
                chat_id: chat(),
                ts: 10,
                thread: None,
            },
            json!({ "chatId": "c1", "ts": 10, "thread": null }),
        );
        pinned(
            ReactionArgs {
                chat_id: chat(),
                ts: 10,
                kind: 3,
                add: false,
                thread: None,
            },
            json!({ "chatId": "c1", "ts": 10, "kind": 3, "add": false, "thread": null }),
        );
        pinned(
            ImportantArgs {
                chat_id: chat(),
                ts: 10,
                important: true,
                thread: None,
            },
            json!({ "chatId": "c1", "ts": 10, "important": true, "thread": null }),
        );
        pinned(
            UnreadArgs {
                chat_id: chat(),
                ts: 10,
                seq_no: 4,
            },
            json!({ "chatId": "c1", "ts": 10, "seqNo": 4 }),
        );
        pinned(
            PinArgs {
                chat_id: chat(),
                ts: Some(10),
            },
            json!({ "chatId": "c1", "ts": 10 }),
        );
        // Unpinning: the UI leaves `ts` out.
        let unpin: PinArgs = serde_json::from_value(json!({ "chatId": "c1" })).unwrap();
        assert_eq!(
            unpin,
            PinArgs {
                chat_id: chat(),
                ts: None
            }
        );
    }

    /// The arguments of the file, people and chat commands, key by key.
    #[test]
    fn file_people_and_chat_args_are_pinned() {
        pinned(
            UrlArgs {
                url: "https://telemost.yandex.ru/j/1".into(),
            },
            json!({ "url": "https://telemost.yandex.ru/j/1" }),
        );
        pinned(
            PathArgs {
                path: "kept:1/a.png".into(),
            },
            json!({ "path": "kept:1/a.png" }),
        );
        pinned(
            SendFilesArgs {
                chat_id: chat(),
                thread: Some(5),
                paths: vec!["/tmp/a.png".into()],
                text: "".into(),
                upload: Some("up1".into()),
                options: SendOptions::default(),
            },
            json!({ "chatId": "c1", "thread": 5, "paths": ["/tmp/a.png"], "text": "", "upload": "up1",
                "options": { "silent": false, "important": false, "follows": false } }),
        );
        pinned(
            UploadFileArgs {
                upload: "up1".into(),
                path: "/tmp/a.png".into(),
            },
            json!({ "upload": "up1", "path": "/tmp/a.png" }),
        );
        pinned(GuidArgs { guid: guid() }, json!({ "guid": "u1" }));
        pinned(
            PeopleSearchArgs {
                query: "ann".into(),
                chat_id: None,
            },
            json!({ "query": "ann", "chatId": null }),
        );
        pinned(
            DirectorySearchArgs {
                query: "dev".into(),
                org: Some(7),
            },
            json!({ "query": "dev", "org": 7 }),
        );
        pinned(
            CreateChannelArgs {
                name: "dev".into(),
                description: "".into(),
                members: vec![guid()],
                public: true,
            },
            json!({ "name": "dev", "description": "", "members": ["u1"], "public": true }),
        );
        pinned(
            MembersArgs {
                chat_id: chat(),
                guids: vec![guid()],
                add: false,
            },
            json!({ "chatId": "c1", "guids": ["u1"], "add": false }),
        );
        pinned(
            UpdateChatArgs {
                chat_id: chat(),
                name: "dev".into(),
                description: "team".into(),
            },
            json!({ "chatId": "c1", "name": "dev", "description": "team" }),
        );
    }

    /// The arguments of the chat list, presence, call, window, emoji and update commands, key by key.
    #[test]
    fn settings_and_window_args_are_pinned() {
        pinned(
            NotifyLevelArgs {
                chat_id: chat(),
                level: NotifyLevel::Mentions,
            },
            json!({ "chatId": "c1", "level": "mentions" }),
        );
        pinned(
            StarArgs {
                chat_id: chat(),
                starred: true,
            },
            json!({ "chatId": "c1", "starred": true }),
        );
        pinned(
            FoldersArgs {
                org: "7".into(),
                folders: vec![ChatFolder {
                    id: "f1".into(),
                    org: "7".into(),
                    name: "Work".into(),
                    chats: vec![chat()],
                }],
            },
            json!({
                "org": "7",
                "folders": [{ "id": "f1", "org": "7", "name": "Work", "chats": ["c1"] }],
            }),
        );
        pinned(GuidsArgs { guids: vec![guid()] }, json!({ "guids": ["u1"] }));
        pinned(
            StatusArgs {
                availability: 2,
                emoji: "🌴".into(),
                text: "away".into(),
                duration: Some(3600),
            },
            json!({ "availability": 2, "emoji": "🌴", "text": "away", "duration": 3600 }),
        );
        pinned(AnswerArgs { accept: true }, json!({ "accept": true }));
        pinned(SoundArgs { id: "bell".into() }, json!({ "id": "bell" }));
        pinned(
            TitleArgs {
                title: "Mostik (3)".into(),
            },
            json!({ "title": "Mostik (3)" }),
        );
        pinned(ZoomArgs { scale: 1.25 }, json!({ "scale": 1.25 }));
        pinned(
            DeleteEmojiArgs {
                chat_id: chat(),
                ts: 10,
            },
            json!({ "chatId": "c1", "ts": 10 }),
        );
        pinned(AutoUpdateArgs { enabled: false }, json!({ "enabled": false }));
        pinned(NotifyMode::All, json!("all"));
        pinned(NotifyMode::Nothing, json!("nothing"));
    }

    /// The name is the type's name in snake case, so two swapped names (`LeaveChat` named
    /// "join_chat") fail here.
    #[test]
    fn names_spell_their_types() {
        for c in ALL {
            let mut snake = String::new();
            for (i, ch) in c.ty.chars().enumerate() {
                if ch.is_ascii_uppercase() && i > 0 {
                    snake.push('_');
                }
                snake.push(ch.to_ascii_lowercase());
            }
            assert_eq!(c.name, snake, "{}", c.ty);
        }
        assert_eq!(EditMessage::NAME, "edit_message");
    }
}
