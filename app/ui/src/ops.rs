//! UI operations. In live mode they go through the Tauri backend, in demo mode they stay local.

mod calls;
mod chats;
mod events;
mod links;
mod messages;
mod people;
mod session;
mod sidebar;
mod snapshot;
mod threads;

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::app::Ctx;
use crate::bridge;

pub use calls::{call_user, leave_call, start_call};
pub use chats::{jump_to_message, leave_chat, load_older, mark_chat_read, navigate, open_chat, recount_soon};
pub use links::{message_link, open_url};
pub use messages::{
    ReplyTarget, delete, edit, forward, mark_unread, pin_message, send, send_files, start_reply, toggle_important,
    toggle_reaction, typing,
};
pub use people::{expire_presence, message_user, open_profile, refresh_me, refresh_presence, set_status};
pub use session::{apply_boot, install_update, sign_out, subscribe};
pub use sidebar::{change_folders, hide_chat, is_starred, set_notify_level, toggle_star};
pub use snapshot::add_channel;
// Only the demo (`app::show_demo`) loads the followed threads from outside.
#[cfg(feature = "demo")]
pub use threads::load_threads;
pub use threads::{open_followed_thread, open_thread};

/// Shows an error in the error bar and the console.
fn report(c: Ctx, e: String) {
    leptos::logging::warn!("{e}");
    c.error.set(Some(e));
}

/// Shows "`what`: `e`" in the error bar and the console: the action `what` failed with `e`.
pub fn failed(c: Ctx, what: &str, e: impl std::fmt::Display) {
    report(c, format!("{what}: {e}"));
}

/// Now, in the microseconds of message times.
pub fn now_mcs() -> Timestamp {
    (js_sys::Date::now() * 1000.0) as Timestamp
}

/// Authors that came with a history page: new ones are added, a known one keeps its data and only
/// gets what it lacked (a push may have brought the bare `From` without a photo).
fn learn_users(s: &mut Snapshot, users: Vec<User>) {
    for u in users {
        match s.users.iter_mut().find(|x| x.id == u.id) {
            Some(known) => {
                if !known.has_name() && u.has_name() {
                    known.display_name = u.display_name;
                }
                known.avatar = known.avatar.take().or(u.avatar);
                known.title = known.title.take().or(u.title);
            }
            None => s.users.push(u),
        }
    }
}

/// A random number, for new ids.
pub fn random_u64() -> u64 {
    let half = || (js_sys::Math::random() * 4_294_967_296.0) as u64;
    (half() << 32) | half()
}

/// Plays a notification sound; the demo in a browser stays silent.
pub fn play_sound(c: Ctx, id: String) {
    if c.live {
        spawn_local(async move {
            let _ = bridge::play_sound(&id).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::chats::{older_page_before, page_start};
    use super::links::split_message_link;
    use super::snapshot::add_new;
    use super::*;

    fn page(messages: Vec<Message>, oldest: Option<Timestamp>) -> bridge::HistoryPage {
        bridge::HistoryPage {
            messages,
            users: vec![],
            more: true,
            oldest,
            peer_seen: 0,
        }
    }

    /// The demo data with only the given messages in chat `general`.
    fn snapshot(messages: Vec<Message>) -> Snapshot {
        let mut s = mostik_core::mock::demo(1_794_300_000_000_000);
        s.messages = messages;
        for ch in &mut s.chats {
            (ch.unread, ch.mentions) = (0, 0);
        }
        s
    }

    fn message(ts: Timestamp, text: &str) -> Message {
        Message {
            chat_id: ChatId("general".into()),
            ts,
            author: UserId("anna".into()),
            text: text.into(),
            thread_parent: None,
            edited: false,
            important: false,
            reactions: vec![],
            thread: None,
            seq_no: 0,
            attachments: vec![],
            forwarded: vec![],
            call: None,
            system: None,
            reads: None,
        }
    }

    fn counters(s: &Snapshot) -> (u32, u32) {
        let ch = s.chat(&ChatId("general".into())).unwrap();
        (ch.unread, ch.mentions)
    }

    #[test]
    fn a_page_of_deleted_records_moves_the_history_start() {
        // 30 deleted messages: no message, but the oldest record is further back.
        assert_eq!(page_start(Some(1000), &page(vec![], Some(400))), Some(400));
        assert_eq!(page_start(None, &page(vec![message(700, "a")], Some(400))), Some(400));
        // A backend without `oldest`: the oldest message.
        assert_eq!(page_start(Some(1000), &page(vec![message(700, "a")], None)), Some(700));
        // Nothing older than asked for: the history goes no further.
        assert_eq!(page_start(Some(1000), &page(vec![], None)), None);
        assert_eq!(page_start(Some(1000), &page(vec![], Some(1000))), None);
    }

    #[test]
    fn older_pages_start_where_the_loaded_history_starts() {
        let id = ChatId("general".into());
        let s = snapshot(vec![message(700, "a"), message(900, "b")]);
        // After a run of deleted messages the start is before the oldest message shown.
        assert_eq!(older_page_before(&s, &id, Some(400)), Some(400));
        assert_eq!(older_page_before(&s, &id, None), Some(700));
        assert_eq!(older_page_before(&s, &ChatId("dev".into()), None), None);
    }

    #[test]
    fn an_edit_of_an_old_message_is_no_news() {
        let edited = |ts, text| Message {
            edited: true,
            ..message(ts, text)
        };
        let mut s = snapshot(vec![message(700, "a"), message(900, "b")]);
        // Older than the loaded part: not shown, not counted.
        assert!(!add_new(&mut s, &edited(500, "@all fixed"), false, None));
        assert!(!add_new(&mut s, &edited(650, "@all fixed"), false, Some(680)));
        assert_eq!(s.messages.len(), 2);
        assert_eq!(counters(&s), (0, 0));
        // Within the loaded part but not loaded (a gap): shown, still not counted.
        assert!(add_new(&mut s, &edited(800, "@all fixed"), false, None));
        assert_eq!(counters(&s), (0, 0));
        // A new message counts, a mention too.
        assert!(add_new(&mut s, &message(1000, "@all news"), false, None));
        assert_eq!(counters(&s), (1, 1));
        // Not in a chat the user is looking at.
        assert!(add_new(&mut s, &message(1100, "@all more"), true, None));
        assert_eq!(counters(&s), (1, 1));
    }

    #[test]
    fn an_edit_of_a_thread_reply_adds_no_reply() {
        let mut parent = message(700, "question");
        parent.thread = Some(ThreadSummary {
            reply_count: 3,
            last_reply_ts: 760,
            participants: vec![],
        });
        let reply = |ts, edited| Message {
            thread_parent: Some(700),
            edited,
            ..message(ts, "answer")
        };
        let mut s = snapshot(vec![parent, reply(750, false)]);
        let count = |s: &Snapshot| s.messages[0].thread.as_ref().map(|t| t.reply_count);
        assert!(!add_new(&mut s, &reply(720, true), false, None));
        assert!(add_new(&mut s, &reply(755, true), false, None));
        assert_eq!(count(&s), Some(3));
        assert!(add_new(&mut s, &reply(780, false), false, None));
        assert_eq!(count(&s), Some(4));
        assert_eq!(counters(&s), (0, 0));
    }

    #[test]
    fn splits_message_links() {
        let own = "https://messenger.360.yandex.ru/chats/0%2F0%2Fabc/123";
        assert_eq!(split_message_link(own), Some(("0%2F0%2Fabc", 123)));
        assert_eq!(
            split_message_link("https://telemost.360.yandex.ru/chat/#/chats/a_b/7/"),
            Some(("a_b", 7))
        );
        assert_eq!(
            split_message_link("https://messenger.360.yandex.ru/#/chats/x/9"),
            Some(("x", 9))
        );
        assert_eq!(
            split_message_link("https://messenger.360.yandex.ru/chats/x/notanumber"),
            None
        );
        assert_eq!(split_message_link("https://example.com/chats/x/1"), None);
    }
}
