//! Layout of a feed: which messages make lines, in which day, and which lines continue the group
//! above. Pure, so that the UI can key its lines by message and rebuild only those that changed.

use crate::model::*;

/// One line of a feed: a message or a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedLine {
    pub msg: Message,
    /// The line continues the group above it (same author, a few minutes apart, same day):
    /// it shows no avatar and name.
    pub grouped: bool,
    /// A call line: the note of the call's end once it ended. A paired end note has no line.
    pub call_end: Option<Message>,
    /// A call line of the latest call in the feed, the only one that may offer "Join".
    pub live_call: bool,
}

/// The lines of one calendar day, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedDay<D> {
    pub day: D,
    pub lines: Vec<FeedLine>,
}

/// Lays out `list` (a chat feed or the replies of a thread, oldest first) by calendar day;
/// `day` tells the day of a time.
///
/// A call is one line, as in Slack: its start note shows the end too (how it went, how long it
/// lasted), and the end note itself is not shown. An end without a start in the list (a direct
/// call in the history, whose ringing Telemost does not keep) is a line itself.
pub fn layout<D: PartialEq>(list: &[Message], day: impl Fn(Timestamp) -> D) -> Vec<FeedDay<D>> {
    let mut ends: std::collections::HashMap<Timestamp, &Message> = Default::default();
    let mut paired: std::collections::HashSet<Timestamp> = Default::default();
    let mut open: std::collections::HashMap<&str, Timestamp> = Default::default();
    for m in list {
        match &m.call {
            Some(note @ (CallNote::Started { .. } | CallNote::Ringing { .. })) => {
                open.insert(note.url(), m.ts);
            }
            Some(note @ CallNote::Ended { .. }) => {
                // The first call of a group call chat starts without a link.
                if let Some(start) = open.remove(note.url()).or_else(|| open.remove("")) {
                    ends.insert(start, m);
                    paired.insert(m.ts);
                }
            }
            None => {}
        }
    }
    let last_call = list.iter().rev().find(|m| m.call.is_some()).map(|m| m.ts);
    let mut out: Vec<FeedDay<D>> = Vec::new();
    let mut prev: Option<&Message> = None;
    for m in list {
        if m.call.is_some() && paired.contains(&m.ts) {
            continue;
        }
        let d = day(m.ts);
        let new_day = out.last().is_none_or(|last| last.day != d);
        let grouped = !new_day && continues_group(prev, m);
        let (call_end, live_call) = match &m.call {
            Some(CallNote::Ended { .. }) => (Some(m.clone()), false),
            Some(_) => {
                let end = ends.get(&m.ts).map(|e| (*e).clone());
                let live = end.is_none() && last_call == Some(m.ts);
                (end, live)
            }
            None => (None, false),
        };
        let line = FeedLine {
            msg: m.clone(),
            grouped,
            call_end,
            live_call,
        };
        match out.last_mut() {
            Some(last) if !new_day => last.lines.push(line),
            _ => out.push(FeedDay {
                day: d,
                lines: vec![line],
            }),
        }
        prev = Some(m);
    }
    out
}

/// The line of message `ts` in a layout.
pub fn find_line<D>(days: &[FeedDay<D>], ts: Timestamp) -> Option<&FeedLine> {
    days.iter().find_map(|d| {
        d.lines
            .binary_search_by_key(&ts, |l| l.msg.ts)
            .ok()
            .map(|i| &d.lines[i])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Timestamp = 86_400_000_000;
    const MIN: Timestamp = 60_000_000;

    fn msg(author: &str, ts: Timestamp) -> Message {
        Message {
            chat_id: ChatId("c".into()),
            ts,
            author: UserId(author.into()),
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
        }
    }

    fn call(ts: Timestamp, url: &str, started: bool) -> Message {
        let url = url.to_owned();
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
            ..msg("u1", ts)
        }
    }

    fn by_day(ts: Timestamp) -> Timestamp {
        ts / DAY
    }

    /// `(ts, grouped)` of every line, day by day.
    fn shape(days: &[FeedDay<Timestamp>]) -> Vec<Vec<(Timestamp, bool)>> {
        days.iter()
            .map(|d| d.lines.iter().map(|l| (l.msg.ts, l.grouped)).collect())
            .collect()
    }

    #[test]
    fn days_split_the_feed_and_break_groups() {
        let list = [
            msg("u1", DAY),
            msg("u1", DAY + MIN),
            msg("u2", DAY + 2 * MIN),
            msg("u2", 2 * DAY - MIN),
            // The same author a minute later, but on the next day: a new group.
            msg("u2", 2 * DAY),
            msg("u2", 2 * DAY + MIN),
        ];
        let days = layout(&list, by_day);
        assert_eq!(days.iter().map(|d| d.day).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(
            shape(&days),
            [
                vec![
                    (DAY, false),
                    (DAY + MIN, true),
                    (DAY + 2 * MIN, false),
                    (2 * DAY - MIN, false)
                ],
                vec![(2 * DAY, false), (2 * DAY + MIN, true)],
            ]
        );
    }

    #[test]
    fn a_call_is_one_line_with_its_end() {
        let list = [
            call(DAY, "a", true),
            msg("u1", DAY + MIN),
            call(DAY + 2 * MIN, "a", false),
            // An end whose start is not loaded is a line itself.
            call(DAY + 3 * MIN, "b", false),
            call(DAY + 4 * MIN, "c", true),
        ];
        let days = layout(&list, by_day);
        let lines = &days[0].lines;
        assert_eq!(
            lines.iter().map(|l| l.msg.ts).collect::<Vec<_>>(),
            [DAY, DAY + MIN, DAY + 3 * MIN, DAY + 4 * MIN]
        );
        assert_eq!(lines[0].call_end.as_ref().map(|e| e.ts), Some(DAY + 2 * MIN));
        assert!(!lines[0].live_call);
        assert_eq!(lines[2].call_end.as_ref().map(|e| e.ts), Some(DAY + 3 * MIN));
        // Only the latest call, still going, can be joined.
        assert!(lines[3].live_call && lines[3].call_end.is_none());
        // A message after a call starts its own group, and so does the call.
        assert!(!lines[1].grouped);
        assert!(!lines[3].grouped);
    }

    #[test]
    fn the_first_call_of_a_call_chat_starts_without_a_link() {
        let list = [call(DAY, "", true), call(DAY + MIN, "a", false)];
        let days = layout(&list, by_day);
        assert_eq!(days[0].lines.len(), 1);
        assert_eq!(days[0].lines[0].call_end.as_ref().map(|e| e.ts), Some(DAY + MIN));
    }

    #[test]
    fn lines_are_found_by_time() {
        let list = [msg("u1", DAY), msg("u1", DAY + MIN), msg("u2", 3 * DAY)];
        let days = layout(&list, by_day);
        assert_eq!(find_line(&days, 3 * DAY).map(|l| l.msg.author.0.as_str()), Some("u2"));
        assert_eq!(find_line(&days, DAY + MIN).map(|l| l.grouped), Some(true));
        assert!(find_line(&days, 2 * DAY).is_none());
    }
}
