//! Read receipts of the user's own messages, in the spirit of Slack: one muted line under a
//! message, "Seen" in a direct chat and "Seen by N" in a group. Telemost counts reads from the
//! `SeenMarker`s people send; it shows them only for the author's own messages.

use crate::model::*;

/// Who has read a message of the user's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadStatus {
    /// A direct chat: whether the peer has seen it and, if known, when (to the minute).
    Seen { seen: bool, at: Option<Timestamp> },
    /// A group: how many people have read it, the latest of them (at most three, newest first),
    /// and whether everyone in the chat has.
    Read {
        count: u32,
        recent: Vec<UserId>,
        everyone: bool,
    },
}

impl ReadStatus {
    /// Someone has read the message.
    pub fn is_read(&self) -> bool {
        match self {
            ReadStatus::Seen { seen, .. } => *seen,
            ReadStatus::Read { count, .. } => *count > 0,
        }
    }
}

/// The read line of a chat: under which own message it goes and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadLine {
    pub ts: Timestamp,
    pub status: ReadStatus,
}

/// What can be told about the reads of message `m` in `chat`; `None` where there are no
/// receipts: others' messages, notes, thread replies, "Notes", channels, and group messages
/// Telemost sent without counters.
pub fn read_status(chat: &Chat, me: &UserId, m: &Message) -> Option<ReadStatus> {
    if &m.author != me || m.is_note() || m.thread_parent.is_some() || m.chat_id != chat.id {
        return None;
    }
    match chat.kind {
        ChatKind::Direct { .. } => {
            let reads = m.reads.as_ref();
            // The peer's position says it, and so do counters of the message itself.
            let seen = m.ts <= chat.peer_seen_ts || reads.is_some_and(|r| r.count > 0 || r.peer_seen.is_some());
            Some(ReadStatus::Seen {
                seen,
                at: reads.and_then(|r| r.peer_seen).filter(|_| seen),
            })
        }
        ChatKind::Group { .. } => m.reads.as_ref().map(|r| ReadStatus::Read {
            count: r.count,
            recent: r.recent.clone(),
            everyone: chat.member_count > 1 && r.count + 1 >= chat.member_count,
        }),
        ChatKind::Notes | ChatKind::Broadcast => None,
    }
}

/// Where the read line of a chat goes: under the latest own message someone has read (`feed` is
/// the chat's feed, oldest first). As the peer reads on, the line moves down.
pub fn read_line(chat: &Chat, me: &UserId, feed: &[Message]) -> Option<ReadLine> {
    feed.iter().rev().find_map(|m| {
        read_status(chat, me, m)
            .filter(ReadStatus::is_read)
            .map(|status| ReadLine { ts: m.ts, status })
    })
}

/// What a read by someone else changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SeenChange {
    /// Something the user may see changed.
    pub changed: bool,
    /// An earlier own message of a group may have been read as well. Telemost does not say: the
    /// counters are to be asked for again with the latest history page.
    pub recount: bool,
}

/// How many readers a message keeps by name, as Telemost's `RecentUserReads`.
const RECENT_READERS: usize = 3;

impl Snapshot {
    /// Someone else read `chat` up to message `ts` (their `SeenMarker`); `reads` are the counters
    /// of that message the push brought.
    pub fn peer_seen(&mut self, chat: &ChatId, user: &UserId, ts: Timestamp, reads: Option<Reads>) -> SeenChange {
        let me = self.me.clone();
        let Some(ch) = self.chats.iter_mut().find(|c| &c.id == chat) else {
            return SeenChange::default();
        };
        let mut change = SeenChange::default();
        let (kind, members) = (ch.kind.clone(), ch.member_count);
        let direct = matches!(kind, ChatKind::Direct { .. });
        // Only a direct chat shows the others' position: a group's moves with every reader.
        let before = ch.peer_seen_ts;
        if ts > before {
            ch.peer_seen_ts = ts;
            change.changed = direct;
        }
        let own = |m: &Message| &m.chat_id == chat && m.thread_parent.is_none() && m.author == me && !m.is_note();
        // A direct chat: own messages the peer has just reached were seen at the time it says.
        let at = reads.as_ref().and_then(|r| r.peer_seen);
        if direct && let Some(at) = at {
            for m in self
                .messages
                .iter_mut()
                .filter(|m| own(m) && m.ts > before && m.ts < ts)
            {
                let r = m.reads.get_or_insert_with(Reads::default);
                if r.peer_seen.is_none() {
                    r.peer_seen = Some(at);
                    change.changed = true;
                }
            }
        }
        if let Some(mut fresh) = reads
            && let Some(m) = self.messages.iter_mut().find(|m| own(m) && m.ts == ts)
        {
            let old = m.reads.take().unwrap_or_default();
            // The push names nobody: the reader goes first among the known ones.
            if fresh.recent.is_empty() {
                fresh.recent = std::iter::once(user.clone())
                    .chain(old.recent.into_iter().filter(|u| u != user))
                    .take(RECENT_READERS)
                    .collect();
            }
            fresh.peer_seen = fresh.peer_seen.or(old.peer_seen);
            m.reads = Some(fresh);
            change.changed = true;
        }
        // A group: the latest own message before `ts` counts this reader only if they had not
        // read it yet, which Telemost does not tell, unless they are named or everyone read it.
        if matches!(kind, ChatKind::Group { .. }) {
            change.recount = self
                .messages
                .iter()
                .filter(|m| own(m) && m.ts < ts)
                .max_by_key(|m| m.ts)
                .is_some_and(|m| match &m.reads {
                    Some(r) => !r.recent.contains(user) && r.count + 1 < members,
                    None => true,
                });
        }
        change
    }

    /// Takes the read counters of `page` (the latest history page of `chat`) for the messages
    /// already loaded, and the position of the others in the chat. A message without counters
    /// in the page keeps those it had.
    pub fn take_reads(&mut self, chat: &ChatId, page: &[Message], peer_seen: Timestamp) {
        if let Some(ch) = self.chats.iter_mut().find(|c| &c.id == chat) {
            ch.peer_seen_ts = ch.peer_seen_ts.max(peer_seen);
        }
        for fresh in page.iter().filter(|m| m.reads.is_some()) {
            if let Some(m) = self
                .messages
                .iter_mut()
                .find(|m| m.chat_id == fresh.chat_id && m.ts == fresh.ts)
            {
                m.reads = fresh.reads.clone();
            }
        }
    }
}

impl Message {
    /// Keeps what this session knows that a fresh copy from Telemost may lack: read counters do
    /// not come with every copy of a message (not with the echo of a sent one, for one).
    pub fn keep_reads(&mut self, old: &Message) {
        if self.reads.is_none() {
            self.reads = old.reads.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "me";

    fn id(s: &str) -> UserId {
        UserId(s.into())
    }

    fn chat(kind: ChatKind, members: u32, peer_seen_ts: Timestamp) -> Chat {
        Chat {
            id: ChatId("c".into()),
            kind,
            name: String::new(),
            description: None,
            member_count: members,
            unread: 0,
            mentions: 0,
            muted: false,
            mute_mentions: false,
            last_ts: 0,
            pinned: None,
            organizations: vec![],
            peer_seen_ts,
        }
    }

    fn direct(peer_seen_ts: Timestamp) -> Chat {
        chat(ChatKind::Direct { peer: id("peer") }, 2, peer_seen_ts)
    }

    fn group(members: u32) -> Chat {
        chat(ChatKind::Group { public: true }, members, 0)
    }

    fn msg(author: &str, ts: Timestamp) -> Message {
        Message {
            chat_id: ChatId("c".into()),
            ts,
            author: id(author),
            text: "text".into(),
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

    fn read(count: u32, recent: &[&str]) -> Option<Reads> {
        Some(Reads {
            count,
            recent: recent.iter().map(|r| id(r)).collect(),
            peer_seen: None,
        })
    }

    fn with_reads(mut m: Message, reads: Option<Reads>) -> Message {
        m.reads = reads;
        m
    }

    fn snapshot(chat: Chat, messages: Vec<Message>) -> Snapshot {
        Snapshot {
            me: id(ME),
            organization: String::new(),
            organization_avatar: None,
            organizations: vec![],
            users: vec![],
            chats: vec![chat],
            sections: vec![],
            messages,
            folders: vec![],
            paused: None,
        }
    }

    #[test]
    fn direct_line_goes_under_the_last_message_the_peer_saw() {
        let call = Message {
            call: Some(CallNote::Started { url: "u".into() }),
            ..msg(ME, 25)
        };
        let feed = [
            with_reads(
                msg(ME, 10),
                Some(Reads {
                    peer_seen: Some(12),
                    ..Default::default()
                }),
            ),
            msg(ME, 20),
            msg("peer", 22),
            call,
            msg(ME, 30),
        ];
        // The peer read up to 26: the last own message is unread, the call note is skipped.
        let line = read_line(&direct(26), &id(ME), &feed).unwrap();
        assert_eq!(line.ts, 20);
        assert_eq!(line.status, ReadStatus::Seen { seen: true, at: None });
        assert_eq!(
            read_line(&direct(15), &id(ME), &feed).map(|l| l.status),
            Some(ReadStatus::Seen {
                seen: true,
                at: Some(12)
            })
        );
        // Nothing the peer saw among the loaded messages: no line.
        assert_eq!(read_line(&direct(5), &id(ME), &feed[1..]), None);
        assert_eq!(
            read_status(&direct(5), &id(ME), &feed[4]),
            Some(ReadStatus::Seen { seen: false, at: None })
        );
        // Others' messages have no receipts.
        assert_eq!(read_status(&direct(26), &id(ME), &feed[2]), None);
    }

    #[test]
    fn group_line_goes_under_the_last_message_someone_read() {
        let feed = [
            with_reads(msg(ME, 10), read(5, &["a", "b", "c"])),
            with_reads(msg(ME, 20), read(2, &["a", "b"])),
            with_reads(msg(ME, 30), read(0, &[])),
            // Telemost sent no counters: nothing to say.
            msg(ME, 40),
        ];
        let line = read_line(&group(6), &id(ME), &feed).unwrap();
        assert_eq!(line.ts, 20);
        assert_eq!(
            line.status,
            ReadStatus::Read {
                count: 2,
                recent: vec![id("a"), id("b")],
                everyone: false
            }
        );
        assert_eq!(
            read_status(&group(6), &id(ME), &feed[0]),
            Some(ReadStatus::Read {
                count: 5,
                recent: vec![id("a"), id("b"), id("c")],
                everyone: true
            })
        );
        assert_eq!(read_status(&group(6), &id(ME), &feed[3]), None);
        assert!(!read_status(&group(6), &id(ME), &feed[2]).unwrap().is_read());
    }

    #[test]
    fn no_receipts_in_notes_channels_and_threads() {
        let m = with_reads(msg(ME, 10), read(3, &["a"]));
        assert_eq!(read_status(&chat(ChatKind::Notes, 1, 99), &id(ME), &m), None);
        assert_eq!(read_status(&chat(ChatKind::Broadcast, 50, 99), &id(ME), &m), None);
        let reply = Message {
            thread_parent: Some(5),
            ..m.clone()
        };
        assert_eq!(read_status(&group(6), &id(ME), &reply), None);
        assert_eq!(read_line(&group(6), &id(ME), &[reply]), None);
    }

    #[test]
    fn peer_seen_moves_forward_only() {
        let mut s = snapshot(direct(50), vec![msg(ME, 40)]);
        let change = s.peer_seen(&ChatId("c".into()), &id("peer"), 30, None);
        assert_eq!(s.chats[0].peer_seen_ts, 50);
        assert_eq!(change, SeenChange::default());
        assert!(s.peer_seen(&ChatId("c".into()), &id("peer"), 60, None).changed);
        assert_eq!(s.chats[0].peer_seen_ts, 60);
        // An unknown chat changes nothing.
        assert!(!s.peer_seen(&ChatId("x".into()), &id("peer"), 90, None).changed);
        // Nor does a group read without counters of an own message: nothing shows it.
        let mut s = snapshot(group(5), vec![msg("x", 40)]);
        assert_eq!(
            s.peer_seen(&ChatId("c".into()), &id("a"), 40, None),
            SeenChange::default()
        );
    }

    #[test]
    fn direct_read_times_the_messages_it_reaches() {
        let mut s = snapshot(direct(15), vec![msg(ME, 10), msg(ME, 20), msg(ME, 30), msg(ME, 40)]);
        let push = Reads {
            count: 1,
            recent: vec![],
            peer_seen: Some(35),
        };
        let change = s.peer_seen(&ChatId("c".into()), &id("peer"), 30, Some(push));
        assert!(change.changed && !change.recount);
        let seen: Vec<_> = s
            .messages
            .iter()
            .map(|m| m.reads.as_ref().and_then(|r| r.peer_seen))
            .collect();
        assert_eq!(seen, [None, Some(35), Some(35), None]);
        assert_eq!(
            s.messages[2].reads.as_ref().map(|r| r.recent.clone()),
            Some(vec![id("peer")])
        );
    }

    #[test]
    fn group_read_names_the_reader_and_asks_for_earlier_messages() {
        let c = ChatId("c".into());
        let mut s = snapshot(
            group(10),
            vec![
                with_reads(msg(ME, 10), read(3, &["a", "b", "c"])),
                msg("x", 20),
                with_reads(msg(ME, 30), read(1, &["a"])),
            ],
        );
        // Counters for message 30 itself: the reader goes first, without repeats.
        let change = s.peer_seen(&c, &id("b"), 30, read(2, &[]));
        assert_eq!(s.messages[2].reads, read(2, &["b", "a"]));
        // Message 10 already names "b": nothing to ask.
        assert_eq!(
            change,
            SeenChange {
                changed: true,
                recount: false
            }
        );
        // "d" read past message 30: whether they had read it before is unknown.
        let change = s.peer_seen(&c, &id("d"), 40, None);
        assert!(change.recount);
        // A reader already named needs no recount.
        assert!(!s.peer_seen(&c, &id("a"), 40, None).recount);
    }

    #[test]
    fn counters_are_kept_when_a_copy_has_none() {
        let old = with_reads(msg(ME, 10), read(2, &["a"]));
        let mut echo = msg(ME, 10);
        echo.keep_reads(&old);
        assert_eq!(echo.reads, read(2, &["a"]));
        let mut fresh = with_reads(msg(ME, 10), read(3, &["b", "a"]));
        fresh.keep_reads(&old);
        assert_eq!(fresh.reads, read(3, &["b", "a"]));

        let mut s = snapshot(group(5), vec![old, with_reads(msg(ME, 20), read(1, &["a"]))]);
        let page = [msg(ME, 10), with_reads(msg(ME, 20), read(4, &["c", "a"])), msg(ME, 30)];
        s.take_reads(&ChatId("c".into()), &page, 25);
        assert_eq!(s.messages[0].reads, read(2, &["a"]));
        assert_eq!(s.messages[1].reads, read(4, &["c", "a"]));
        assert_eq!(s.messages.len(), 2, "only counters are taken");
        assert_eq!(s.chats[0].peer_seen_ts, 25);
    }
}
