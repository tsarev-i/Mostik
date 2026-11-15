//! The "Files" tab of a chat, as in Slack: only the attachments of its messages, files and
//! pictures, newest first, with who sent each and when, a filter by kind and a search by name. A
//! click opens the file (a picture in the viewer, a file is saved as from its card in the chat);
//! "In chat" goes to its message. Telemost has no list of a chat's files: the tab shows what is
//! loaded of the chat and loads older messages on request.

use leptos::prelude::*;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::{bridge, ops, time};

use super::message::{human_size, save_attachment};
use super::state::{ChatTab, ctx};

/// What the tab shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    All,
    Images,
    Files,
}

/// An attachment in the list, with its message.
#[derive(Clone, PartialEq)]
struct Row {
    file: Attachment,
    ts: Timestamp,
    thread_parent: Option<Timestamp>,
    author: String,
}

/// The attachments of chat `id` among `messages`, newest first, of kind `kind` and with `query`
/// (lowercase) in their name. Forwarded and quoted files are left out: they belong to other messages.
fn rows(s: &Snapshot, id: &ChatId, kind: Kind, query: &str) -> Vec<Row> {
    let mut out: Vec<Row> = s
        .messages
        .iter()
        .filter(|m| &m.chat_id == id)
        .flat_map(|m| m.attachments.iter().map(move |a| (m, a)))
        .filter(|(_, a)| match kind {
            Kind::All => true,
            Kind::Images => a.kind == AttachmentKind::Image,
            Kind::Files => a.kind == AttachmentKind::File,
        })
        .filter(|(_, a)| query.is_empty() || a.name.to_lowercase().contains(query))
        .map(|(m, a)| Row {
            file: a.clone(),
            ts: m.ts,
            thread_parent: m.thread_parent,
            author: s.user(&m.author).map(|u| u.display_name.clone()).unwrap_or_default(),
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.ts));
    out
}

#[component]
pub(super) fn FilesView() -> impl IntoView {
    let c = ctx();
    let kind = RwSignal::new(Kind::All);
    let query = RwSignal::new(String::new());
    let list = Memo::new(move |_| {
        let id = c.current.get();
        let q = query.with(|q| q.trim().to_lowercase());
        let k = kind.get();
        c.snap.with(|s| rows(s, &id, k, &q))
    });
    let more = move || {
        let id = c.current.get();
        c.live && !c.exhausted.with(|e| e.contains(&id))
    };
    let loading = move || c.current.with(|id| c.loading_older.with(|l| l.contains(id)));
    // Older pages asked here are asked again after a failure only while the tab shows the chat,
    // as in the message list: another chat or another tab stops that.
    let shown = std::sync::Arc::new(std::sync::Mutex::new(None::<ChatId>));
    Effect::new({
        let shown = shown.clone();
        move |_| {
            let id = c.current.get();
            let prev = shown.lock().ok().and_then(|mut s| s.replace(id.clone()));
            if let Some(prev) = prev.filter(|p| *p != id) {
                ops::stop_older(c, &prev);
            }
        }
    });
    on_cleanup(move || {
        if let Some(id) = shown.lock().ok().and_then(|mut s| s.take()) {
            ops::stop_older(c, &id);
        }
    });
    let filter = move |k: Kind, label: &'static str| {
        view! {
            <button class="files-filter" class:active=move || kind.get() == k on:click=move |_| kind.set(k)>{label}</button>
        }
    };
    view! {
        <div class="messages files-view">
            <div class="files-tools">
                <label class="files-search">
                    <Icon icon=I::Search size=14/>
                    <input type="text" placeholder=tr().files_search spellcheck="false" autocomplete="off"
                        prop:value=move || query.get() on:input=move |e| query.set(event_target_value(&e))/>
                </label>
                {filter(Kind::All, tr().files_all)}
                {filter(Kind::Images, tr().files_images)}
                {filter(Kind::Files, tr().files_documents)}
            </div>
            {move || {
                let rows = list.get();
                if rows.is_empty() {
                    return view! { <div class="empty-state"><h3>{tr().files_none}</h3></div> }.into_any();
                }
                rows.into_iter().map(|r| view! { <FileRow row=r/> }).collect_view().into_any()
            }}
            <div class="files-more">
                {move || if loading() {
                    view! { <div class="spinner small"></div> }.into_any()
                } else if more() {
                    view! { <button class="btn small" on:click=move |_| { ops::load_older(c); }>{tr().files_older}</button> }.into_any()
                } else {
                    view! { <span class="files-end">{tr().files_all_loaded}</span> }.into_any()
                }}
            </div>
        </div>
    }
}

/// One attachment: its icon or preview, name, size, who sent it and when, and "In chat".
#[component]
fn FileRow(row: Row) -> impl IntoView {
    let c = ctx();
    let Row {
        file,
        ts,
        thread_parent,
        author,
    } = row;
    let image = file.kind == AttachmentKind::Image;
    let icon = if image && c.live {
        view! { <img class="files-thumb" src=bridge::file_url(&file.file_id, Some("middle")) alt="" loading="lazy"/> }
            .into_any()
    } else {
        view! { <span class="file-icon"><Icon icon=I::Files size=20/></span> }.into_any()
    };
    let open = {
        let file = file.clone();
        move |_| {
            if image {
                c.lightbox.set(Some(file.clone()));
            } else {
                save_attachment(c, file.file_id.clone(), file.name.clone());
            }
        }
    };
    let go = move |_| {
        c.chat_tab.set(ChatTab::Messages);
        let chat = c.current.get_untracked();
        ops::jump_to_message(c, chat, ts, thread_parent);
    };
    let sub = format!("{} · {author} · {}", human_size(file.size), time::short_date(ts));
    view! {
        <div class="files-row">
            <button class="files-open" title=if image { tr().open_btn } else { tr().download } on:click=open>
                {icon}
                <span class="dm-body">
                    <span class="dm-name">{file.name.clone()}</span>
                    <span class="dm-preview">{sub}</span>
                </span>
            </button>
            <button class="link" on:click=go>{tr().view_in_chat}</button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_the_chat_attachments_newest_first() {
        let mut s = mostik_core::mock::demo(1_794_300_000_000_000);
        let chat = s.chats[0].id.clone();
        let other = s.chats[1].id.clone();
        let file = |name: &str, kind| Attachment {
            kind,
            file_id: format!("id-{name}"),
            name: name.into(),
            size: 10,
            width: 0,
            height: 0,
        };
        let template = s.messages[0].clone();
        let message = |chat: &ChatId, ts, attachments| Message {
            chat_id: chat.clone(),
            ts,
            attachments,
            ..template.clone()
        };
        s.messages = vec![
            message(&chat, 10, vec![file("Report.pdf", AttachmentKind::File)]),
            message(
                &chat,
                30,
                vec![
                    file("shot.png", AttachmentKind::Image),
                    file("notes.txt", AttachmentKind::File),
                ],
            ),
            message(&other, 40, vec![file("elsewhere.txt", AttachmentKind::File)]),
            message(&chat, 20, vec![]),
        ];
        let names = |kind, query: &str| -> Vec<String> {
            rows(&s, &chat, kind, query).into_iter().map(|r| r.file.name).collect()
        };
        assert_eq!(names(Kind::All, ""), ["shot.png", "notes.txt", "Report.pdf"]);
        assert_eq!(names(Kind::Images, ""), ["shot.png"]);
        assert_eq!(names(Kind::Files, ""), ["notes.txt", "Report.pdf"]);
        // The query comes lowercase; names are matched in any case.
        assert_eq!(names(Kind::All, "report"), ["Report.pdf"]);
        assert!(names(Kind::Images, "report").is_empty());
    }
}
