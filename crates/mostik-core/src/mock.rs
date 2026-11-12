//! Demo data for layout work and for running the UI in a browser without Tauri.
//! All people and conversations are fictional.

use crate::model::*;

const MIN: Timestamp = 60 * 1_000_000;
const DAY: Timestamp = 24 * 60 * MIN;

fn uid(s: &str) -> UserId {
    UserId(s.to_owned())
}

fn cid(s: &str) -> ChatId {
    ChatId(s.to_owned())
}

fn user(id: &str, name: &str, title: &str, presence: Presence) -> User {
    User {
        title: Some(title.to_owned()),
        presence,
        ..User::named(uid(id), name)
    }
}

fn chat(id: &str, kind: ChatKind, name: &str, members: u32, unread: u32, mentions: u32, last_ts: Timestamp) -> Chat {
    Chat {
        name: name.to_owned(),
        member_count: members,
        unread,
        mentions,
        last_ts,
        ..Chat::new(cid(id), kind)
    }
}

fn msg(chat: &str, ts: Timestamp, author: &str, text: &str) -> Message {
    Message {
        chat_id: cid(chat),
        ts,
        author: uid(author),
        text: text.to_owned(),
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

fn reply(chat: &str, parent: Timestamp, ts: Timestamp, author: &str, text: &str) -> Message {
    Message {
        thread_parent: Some(parent),
        ..msg(chat, ts, author, text)
    }
}

fn call(chat: &str, ts: Timestamp, author: &str, started: bool) -> Message {
    let url = format!("https://telemost.360.yandex.ru/j/demo-{chat}");
    Message {
        call: Some(if started {
            CallNote::Started { url }
        } else {
            CallNote::Ended {
                url,
                reason: None,
                seconds: None,
            }
        }),
        ..msg(chat, ts, author, "")
    }
}

fn note(chat: &str, ts: Timestamp, author: &str, system: SystemNote) -> Message {
    Message {
        system: Some(system),
        ..msg(chat, ts, author, "")
    }
}

fn react(emoji: &str, count: u32, mine: bool) -> Reaction {
    let kind = match emoji {
        "👍" => 100102,
        "🎉" => 100123,
        "👀" => 100110,
        _ => 0,
    };
    Reaction {
        kind,
        emoji: emoji.to_owned(),
        count,
        mine,
        users: vec![],
    }
}

/// Demo snapshot as of `now` (µs since the Unix epoch): one organization, a few channels,
/// direct chats and a thread. The day's messages span the last hour and a half, the newest five
/// minutes old; the rest are from the day before.
pub fn demo(now: Timestamp) -> Snapshot {
    let today = now.saturating_sub(80 * MIN);
    let yesterday = today.saturating_sub(DAY);
    let mut me = user("me", "Ivan Petrov", "Developer", Presence::Online);
    me.status = Some(CustomStatus {
        emoji: "💻".into(),
        text: "Writing a client".into(),
        in_meeting: false,
    });
    let mut anna = user("anna", "Anna Smirnova", "UI designer", Presence::Online);
    anna.status = Some(CustomStatus {
        emoji: "🎨".into(),
        text: "In mockups".into(),
        in_meeting: false,
    });
    let boris = user("boris", "Boris Kotov", "Backend developer", Presence::Away);
    let vera = user("vera", "Vera Lebedeva", "QA engineer", Presence::Online);
    let gleb = user("gleb", "Gleb Orlov", "Product manager", Presence::Offline);
    let bot = User {
        presence: Presence::Online,
        is_robot: true,
        ..User::named(uid("ci-bot"), "Build bot")
    };

    let mut general = chat("general", ChatKind::Group { public: true }, "general", 48, 0, 0, today);
    general.description = Some("News and announcements for the whole team".into());
    let mut dev = chat("dev", ChatKind::Group { public: true }, "dev", 17, 4, 1, today);
    dev.description = Some("Code, reviews and releases".into());
    let design = chat("design", ChatKind::Group { public: true }, "design", 9, 0, 0, today);
    let release = chat(
        "release",
        ChatKind::Group { public: false },
        "release-1-0",
        6,
        2,
        0,
        today,
    );
    let mut random = chat("random", ChatKind::Group { public: true }, "random", 41, 0, 0, today);
    random.muted = true;
    let news = chat("news", ChatKind::Broadcast, "company-news", 312, 1, 0, today);
    let dm_anna = chat("dm-anna", ChatKind::Direct { peer: uid("anna") }, "", 2, 1, 1, today);
    let mut dm_boris = chat("dm-boris", ChatKind::Direct { peer: uid("boris") }, "", 2, 0, 0, today);
    // Boris has read up to the user's question, not the message after it.
    dm_boris.peer_seen_ts = today + 5 * MIN;
    let notes = chat("notes", ChatKind::Notes, "Notes", 1, 0, 0, today);
    // Meeting chats live in their own Telemost namespace, `0/22/…`.
    let sync = chat(
        "0/22/weekly-sync",
        ChatKind::Group { public: false },
        "Weekly sync",
        8,
        1,
        0,
        today + 75 * MIN,
    );
    let demo_call = chat(
        "0/22/release-demo",
        ChatKind::Group { public: false },
        "Release demo",
        12,
        0,
        0,
        today,
    );

    let sections = vec![
        Section {
            id: "starred".into(),
            title: "Starred".into(),
            chats: vec![cid("release")],
        },
        Section {
            id: "channels".into(),
            title: "Channels".into(),
            chats: vec![cid("general"), cid("dev"), cid("design"), cid("random"), cid("news")],
        },
        Section {
            id: "dms".into(),
            title: "Direct messages".into(),
            chats: vec![cid("notes"), cid("dm-anna"), cid("dm-boris")],
        },
        Section {
            id: "meetings".into(),
            title: "Meeting chats".into(),
            chats: vec![cid("0/22/weekly-sync"), cid("0/22/release-demo")],
        },
    ];

    let t_release = today + 12 * MIN;
    let mut release_msg = msg(
        "dev",
        t_release,
        "boris",
        "Deployed build **0.1.0-rc2** to staging. The cache format changed, clear `~/.cache/mostik` before testing.",
    );
    release_msg.reactions = vec![react("👍", 3, true), react("🎉", 2, false)];
    release_msg.thread = Some(ThreadSummary {
        reply_count: 3,
        last_reply_ts: today + 41 * MIN,
        participants: vec![uid("vera"), uid("me"), uid("boris")],
    });

    let mut edited = msg(
        "dev",
        today + 14 * MIN,
        "boris",
        "Ran the migration on a copy of the production database, it took 40 seconds.",
    );
    edited.edited = true;

    // Read receipts: Vera, Anna and Boris read the user's message, everyone else did not yet.
    let mut looked = msg(
        "dev",
        yesterday + 95 * MIN,
        "me",
        "Looked through it, all good. I will pick it up after the login work.",
    );
    looked.reads = Some(Reads {
        count: 5,
        recent: vec![uid("vera"), uid("anna"), uid("boris")],
        peer_seen: None,
    });
    let mut asked = msg("dm-boris", today + 5 * MIN, "me", "Thanks! Which folder?");
    asked.reads = Some(Reads {
        count: 1,
        recent: vec![uid("boris")],
        peer_seen: Some(today + 9 * MIN),
    });

    let mut important = msg(
        "dev",
        today + 63 * MIN,
        "gleb",
        "@Ivan Petrov please look at the login review before lunch, it blocks the release.",
    );
    important.important = true;
    important.reactions = vec![react("👀", 1, true)];

    let messages = vec![
        msg(
            "dev",
            yesterday + 5 * MIN,
            "anna",
            "Updated the sidebar mockup: sections now collapse and unread chats are bold.",
        ),
        msg(
            "dev",
            yesterday + 7 * MIN,
            "anna",
            "The link is in the channel description.",
        ),
        looked,
        release_msg,
        edited,
        msg(
            "dev",
            today + 30 * MIN,
            "ci-bot",
            "Build `main #412` passed: 214 tests, 0 failed.",
        ),
        important,
        reply(
            "dev",
            t_release,
            today + 20 * MIN,
            "vera",
            "Cleared the cache, starting regression testing.",
        ),
        reply(
            "dev",
            t_release,
            today + 33 * MIN,
            "me",
            "Everything starts for me, login works.",
        ),
        reply(
            "dev",
            t_release,
            today + 41 * MIN,
            "boris",
            "Great, then we release tonight.",
        ),
        msg(
            "general",
            yesterday + 60 * MIN,
            "gleb",
            "Release demo on Friday at 16:00. The invite is in the calendar.",
        ),
        msg("general", today + 2 * MIN, "anna", "Good morning, everyone! ☀️"),
        msg(
            "design",
            yesterday + 200 * MIN,
            "anna",
            "We picked Lato for the UI, message text is 15/22.",
        ),
        note(
            "release",
            yesterday + 30 * MIN,
            "gleb",
            SystemNote::Created {
                name: "release-1-0".into(),
            },
        ),
        note(
            "release",
            yesterday + 31 * MIN,
            "gleb",
            SystemNote::Members {
                added: vec![uid("vera"), uid("boris"), uid("me")],
                removed: vec![],
            },
        ),
        note("release", today + 40 * MIN, "anna", SystemNote::Joined),
        msg(
            "release",
            today + 50 * MIN,
            "vera",
            "Regression passed, filed two minor bugs.",
        ),
        Message {
            forwarded: vec![Forwarded {
                author: uid("vera"),
                author_name: "Vera Lebedeva".into(),
                ts: today + 50 * MIN,
                chat_id: Some(cid("release")),
                text: "Regression passed, filed two minor bugs.".into(),
                attachments: vec![],
                quote: Some("filed two minor bugs".into()),
                reply: true,
            }],
            ..msg(
                "release",
                today + 52 * MIN,
                "gleb",
                "Minor ones go into the next patch.",
            )
        },
        call("design", yesterday + 210 * MIN, "anna", true),
        call("design", yesterday + 245 * MIN, "anna", false),
        call("release", today + 55 * MIN, "gleb", true),
        msg("random", today + 8 * MIN, "boris", "Has anyone seen my cat mug?"),
        msg(
            "news",
            today + MIN,
            "gleb",
            "From December 1 the office is open until 21:00.",
        ),
        msg(
            "dm-anna",
            today + 70 * MIN,
            "anna",
            "@Ivan Petrov hi! Could you send a screenshot of the new channel header?",
        ),
        msg(
            "dm-boris",
            yesterday + 300 * MIN,
            "boris",
            "Put the production logs on the shared drive.",
        ),
        asked,
        msg("dm-boris", today + 6 * MIN, "me", "Never mind, found it."),
        msg("notes", yesterday + 10 * MIN, "me", "Check Yandex ID login on Wayland."),
        msg(
            "0/22/weekly-sync",
            today + 75 * MIN,
            "vera",
            "Notes from the sync: login review today, release tonight.",
        ),
        msg(
            "0/22/release-demo",
            yesterday + 62 * MIN,
            "gleb",
            "The demo plan is in the channel description, questions go here.",
        ),
    ];

    Snapshot {
        me: uid("me"),
        organization: "Demo organization".into(),
        organization_avatar: None,
        organizations: vec![Organization {
            id: 1,
            name: "Demo organization".into(),
            avatar: None,
        }],
        users: vec![me, anna, boris, vera, gleb, bot],
        chats: vec![
            general, dev, design, release, random, news, dm_anna, dm_boris, notes, sync, demo_call,
        ],
        sections,
        messages,
        folders: vec![],
        paused: None,
    }
}
