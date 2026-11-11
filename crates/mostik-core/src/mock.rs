//! Demo data for layout work and for running the UI in a browser without Tauri.
//! All people and conversations are fictional.

use crate::model::*;

/// 2026-11-09 10:00 MSK.
const YESTERDAY: Timestamp = 1_794_207_600 * 1_000_000;
/// 2026-11-10 10:00 MSK.
const TODAY: Timestamp = 1_794_294_000 * 1_000_000;
const MIN: Timestamp = 60 * 1_000_000;

fn uid(s: &str) -> UserId {
    UserId(s.to_owned())
}

fn cid(s: &str) -> ChatId {
    ChatId(s.to_owned())
}

fn user(id: &str, name: &str, title: &str, presence: Presence) -> User {
    User {
        id: uid(id),
        display_name: name.to_owned(),
        title: Some(title.to_owned()),
        presence,
        status: None,
        availability: Default::default(),
        is_robot: false,
        avatar: None,
    }
}

fn chat(id: &str, kind: ChatKind, name: &str, members: u32, unread: u32, mentions: u32) -> Chat {
    Chat {
        id: cid(id),
        kind,
        name: name.to_owned(),
        description: None,
        member_count: members,
        unread,
        mentions,
        muted: false,
        mute_mentions: false,
        last_ts: TODAY,
        pinned: None,
        organizations: vec![],
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
            CallNote::Ended { url }
        }),
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

/// Demo snapshot: one organization, a few channels, direct chats and a thread.
pub fn demo() -> Snapshot {
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
        id: uid("ci-bot"),
        display_name: "Build bot".into(),
        title: None,
        presence: Presence::Online,
        status: None,
        availability: Default::default(),
        is_robot: true,
        avatar: None,
    };

    let mut general = chat("general", ChatKind::Group { public: true }, "general", 48, 0, 0);
    general.description = Some("News and announcements for the whole team".into());
    let mut dev = chat("dev", ChatKind::Group { public: true }, "dev", 17, 4, 1);
    dev.description = Some("Code, reviews and releases".into());
    let design = chat("design", ChatKind::Group { public: true }, "design", 9, 0, 0);
    let release = chat("release", ChatKind::Group { public: false }, "release-1-0", 6, 2, 0);
    let mut random = chat("random", ChatKind::Group { public: true }, "random", 41, 0, 0);
    random.muted = true;
    let news = chat("news", ChatKind::Broadcast, "company-news", 312, 1, 0);
    let dm_anna = chat("dm-anna", ChatKind::Direct { peer: uid("anna") }, "", 2, 1, 1);
    let dm_boris = chat("dm-boris", ChatKind::Direct { peer: uid("boris") }, "", 2, 0, 0);
    let notes = chat("notes", ChatKind::Notes, "Notes", 1, 0, 0);
    // Meeting chats live in their own Telemost namespace, `0/22/…`.
    let mut sync = chat(
        "0/22/weekly-sync",
        ChatKind::Group { public: false },
        "Weekly sync",
        8,
        1,
        0,
    );
    sync.last_ts = TODAY + 75 * MIN;
    let demo_call = chat(
        "0/22/release-demo",
        ChatKind::Group { public: false },
        "Release demo",
        12,
        0,
        0,
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

    let t_release = TODAY + 12 * MIN;
    let mut release_msg = msg(
        "dev",
        t_release,
        "boris",
        "Deployed build **0.1.0-rc2** to staging. The cache format changed, clear `~/.cache/mostik` before testing.",
    );
    release_msg.reactions = vec![react("👍", 3, true), react("🎉", 2, false)];
    release_msg.thread = Some(ThreadSummary {
        reply_count: 3,
        last_reply_ts: TODAY + 41 * MIN,
        participants: vec![uid("vera"), uid("me"), uid("boris")],
    });

    let mut edited = msg(
        "dev",
        TODAY + 14 * MIN,
        "boris",
        "Ran the migration on a copy of the production database, it took 40 seconds.",
    );
    edited.edited = true;

    let mut important = msg(
        "dev",
        TODAY + 63 * MIN,
        "gleb",
        "@Ivan Petrov please look at the login review before lunch, it blocks the release.",
    );
    important.important = true;
    important.reactions = vec![react("👀", 1, true)];

    let messages = vec![
        msg(
            "dev",
            YESTERDAY + 5 * MIN,
            "anna",
            "Updated the sidebar mockup: sections now collapse and unread chats are bold.",
        ),
        msg(
            "dev",
            YESTERDAY + 7 * MIN,
            "anna",
            "The link is in the channel description.",
        ),
        msg(
            "dev",
            YESTERDAY + 95 * MIN,
            "me",
            "Looked through it, all good. I will pick it up after the login work.",
        ),
        release_msg,
        edited,
        msg(
            "dev",
            TODAY + 30 * MIN,
            "ci-bot",
            "Build `main #412` passed: 214 tests, 0 failed.",
        ),
        important,
        reply(
            "dev",
            t_release,
            TODAY + 20 * MIN,
            "vera",
            "Cleared the cache, starting regression testing.",
        ),
        reply(
            "dev",
            t_release,
            TODAY + 33 * MIN,
            "me",
            "Everything starts for me, login works.",
        ),
        reply(
            "dev",
            t_release,
            TODAY + 41 * MIN,
            "boris",
            "Great, then we release tonight.",
        ),
        msg(
            "general",
            YESTERDAY + 60 * MIN,
            "gleb",
            "Release demo on Friday at 16:00. The invite is in the calendar.",
        ),
        msg("general", TODAY + 2 * MIN, "anna", "Good morning, everyone! ☀️"),
        msg(
            "design",
            YESTERDAY + 200 * MIN,
            "anna",
            "We picked Lato for the UI, message text is 15/22.",
        ),
        msg(
            "release",
            TODAY + 50 * MIN,
            "vera",
            "Regression passed, filed two minor bugs.",
        ),
        msg(
            "release",
            TODAY + 52 * MIN,
            "gleb",
            "Minor ones go into the next patch.",
        ),
        call("design", YESTERDAY + 210 * MIN, "anna", true),
        call("design", YESTERDAY + 245 * MIN, "anna", false),
        call("release", TODAY + 55 * MIN, "gleb", true),
        msg("random", TODAY + 8 * MIN, "boris", "Has anyone seen my cat mug?"),
        msg(
            "news",
            TODAY + MIN,
            "gleb",
            "From December 1 the office is open until 21:00.",
        ),
        msg(
            "dm-anna",
            TODAY + 70 * MIN,
            "anna",
            "@Ivan Petrov hi! Could you send a screenshot of the new channel header?",
        ),
        msg(
            "dm-boris",
            YESTERDAY + 300 * MIN,
            "boris",
            "Put the production logs on the shared drive.",
        ),
        msg("notes", YESTERDAY + 10 * MIN, "me", "Check Yandex ID login on Wayland."),
        msg(
            "0/22/weekly-sync",
            TODAY + 75 * MIN,
            "vera",
            "Notes from the sync: login review today, release tonight.",
        ),
        msg(
            "0/22/release-demo",
            YESTERDAY + 62 * MIN,
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
    }
}
