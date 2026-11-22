//! A text too long for one message, cut into parts that each go as a message of their own, the
//! way the web client sends it (`enableLongMessageChunking`): a part ends where the text breaks
//! (a blank line, a line, a sentence, a word) and keeps mentions, links and inline code whole; a
//! code block cut in two is closed in one part and opened again in the next.

/// Length as a web page counts it: UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The fence that closes a code block cut at the end of a part.
const CLOSE: &str = "```";

/// A code block, as [`crate::markup::parse_blocks`] finds it: byte offsets of its opening fence,
/// of its code (after the opening line), of its closing fence and of the end of that.
#[derive(Debug)]
struct Fence {
    open: usize,
    body: usize,
    close: usize,
    end: usize,
}

impl Fence {
    /// The opening line a part that goes on with the block starts with: the fence with its
    /// language, if it has one.
    fn opener<'a>(&self, text: &'a str) -> &'a str {
        &text[self.open..self.body]
    }
}

fn fences(text: &str) -> Vec<Fence> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find("```").map(|i| from + i) {
        let Some(close) = text[start + 3..].find("```").map(|i| start + 3 + i) else {
            break;
        };
        // "```" and a newline, or "```lang" and a newline: the code starts after it.
        let line_end = text[start + 3..close].find('\n').map(|i| start + 3 + i);
        let body = match line_end {
            Some(nl) if !text[start + 3..nl].contains(char::is_whitespace) => nl + 1,
            _ => start + 3,
        };
        out.push(Fence {
            open: start,
            body,
            close,
            end: close + 3,
        });
        from = close + 3;
    }
    out
}

/// What a cut must not go through, outside code blocks: `@<guid>` mentions, `[caption](address)`
/// links, bare addresses and `` `code` ``. Byte ranges, in order.
fn kept_whole(text: &str, fences: &[Fence]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut fence = fences.iter().peekable();
    let mut i = 0;
    while i < text.len() {
        if let Some(f) = fence.peek()
            && i >= f.open
        {
            i = f.end;
            fence.next();
            continue;
        }
        let rest = &text[i..];
        let len = if rest.starts_with('@') && crate::markup::guid_at(&rest[1..]).is_some() {
            Some(1 + crate::markup::GUID_LEN)
        } else if rest.starts_with('`') && !rest.starts_with("```") {
            rest[1..]
                .find(['`', '\n'])
                .filter(|&j| rest[1 + j..].starts_with('`'))
                .map(|j| j + 2)
        } else if rest.starts_with('[') {
            link_len(rest)
        } else if ["http://", "https://"].iter().any(|p| rest.starts_with(p)) {
            Some(rest.find(char::is_whitespace).unwrap_or(rest.len()))
        } else {
            None
        };
        match len {
            Some(n) if n > 1 => {
                out.push((i, i + n));
                i += n;
            }
            _ => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    out
}

/// Length of `[caption](address)` at the start of `s`, on one line.
fn link_len(s: &str) -> Option<usize> {
    let line = &s[..s.find('\n').unwrap_or(s.len())];
    let mid = line.find("](")?;
    let close = line[mid + 2..].find(')')?;
    Some(mid + 2 + close + 1)
}

/// The longest a text of one message may be, in UTF-16 code units, as the web client counts it
/// (`messageLengthLimit`): a longer one goes in parts.
pub const MESSAGE_LIMIT: usize = 4096;

/// The longest text the web client sends at all, in parts or not: Telemost refuses a much longer
/// one anyway (`message too big`).
pub const TEXT_MAX: usize = 40_000;

/// How far a hard cut goes back to keep a character whole: farther than that it is not one
/// character but a run of accents or joiners, and is cut anyway.
const MAX_BACK: usize = 32;

/// A flag letter: two of them make one flag.
fn regional(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// Whether `next` belongs to the same character as `prev` on screen: an emoji with its modifier or
/// joiner, a letter with its accent. A cut between them would break it. Flags are counted apart.
fn joins(prev: char, next: char) -> bool {
    let extends = |c: char| {
        matches!(c,
            '\u{200D}' | '\u{FE0E}' | '\u{FE0F}' | '\u{20E3}'
            | '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}')
    };
    extends(next) || prev == '\u{200D}'
}

/// `cut` moved back, not past `pos`, so that it does not split a character on screen: an emoji
/// from its modifier, a letter from its accent, a flag in two.
fn whole_char_cut(text: &str, pos: usize, mut cut: usize) -> usize {
    let start = cut;
    // Flag letters pair from the start of their run: an odd number before the cut means the cut
    // halves a flag.
    if text[cut..].chars().next().is_some_and(regional) {
        let run = text[pos..cut].chars().rev().take_while(|c| regional(*c)).count();
        if run % 2 == 1 && cut - 4 > pos {
            cut -= 4;
        }
    }
    let mut back = 0;
    while let (Some(p), Some(n)) = (text[pos..cut].chars().next_back(), text[cut..].chars().next()) {
        if back == MAX_BACK || cut - p.len_utf8() <= pos || !joins(p, n) {
            break;
        }
        cut -= p.len_utf8();
        back += 1;
    }
    if back == MAX_BACK { start } else { cut }
}

/// Cuts `text` into parts of at most `limit` UTF-16 code units, each to go as a message, in
/// order. A text that fits is one part; an empty one has none.
///
/// A part ends, within its second half, at the last blank line, else the last line break, else
/// the last end of a sentence, else the last space, and never inside a mention, a link or inline
/// code; with none of these it is cut at the limit, between whole characters. The break itself
/// goes. A code block cut in two ends the part with a closing fence and starts the next with its
/// opening line again, so both show as code.
pub fn split_long(text: &str, limit: usize) -> Vec<String> {
    if utf16_len(text) <= limit {
        return if text.is_empty() { vec![] } else { vec![text.to_owned()] };
    }
    // Room for a code block's fences, and some text besides.
    let limit = limit.max(32);
    let fences = fences(text);
    let kept = kept_whole(text, &fences);
    // Whether a part may end at `at`: not in a kept range, a fence's opening line or its closing fence.
    let can_end = |at: usize| {
        let inside = |(a, b): (usize, usize)| a < at && at < b;
        !kept.iter().copied().any(inside)
            && !fences
                .iter()
                .flat_map(|f| [(f.open, f.body), (f.close, f.end)])
                .any(inside)
    };
    let mut parts = Vec::new();
    let mut pos = 0;
    // The opening line of a code block the last part ended inside of.
    let mut reopen: Option<&str> = None;
    while pos < text.len() {
        // An opening line too long to repeat leaves the rest of the block as text.
        let prefix = reopen.filter(|o| utf16_len(o) + 8 <= limit / 2).unwrap_or("");
        let room = limit - utf16_len(prefix);
        if utf16_len(&text[pos..]) <= room {
            parts.push(format!("{prefix}{}", &text[pos..]));
            break;
        }
        // Leaves room for closing a code block: "\n```".
        let room = room - 1 - CLOSE.len();
        let (mut end, mut half, mut units) = (text.len(), text.len(), 0);
        for (i, ch) in text[pos..].char_indices() {
            if units >= room / 2 && half == text.len() {
                half = pos + i;
            }
            units += ch.len_utf16();
            if units > room {
                end = pos + i;
                break;
            }
        }
        // The best break of each kind, the last that may end a part: `(end of the part, start of the next)`.
        let mut best: [Option<(usize, usize)>; 4] = [None; 4];
        let mut prev = '\0';
        for (i, ch) in text[pos..end].char_indices() {
            let at = pos + i;
            if at >= half && ch.is_whitespace() && can_end(at) {
                let next = at + ch.len_utf8();
                if ch == '\n' {
                    // The part ends before the first of the two breaks, which must be a place to end too.
                    if prev == '\n' && at > half && can_end(at - 1) {
                        best[0] = Some((at - 1, next));
                    }
                    best[1] = Some((at, next));
                }
                if matches!(prev, '.' | '!' | '?' | '…') {
                    best[2] = Some((at, next));
                }
                best[3] = Some((at, next));
            }
            prev = ch;
        }
        let (mut cut, mut next) = best.into_iter().flatten().next().unwrap_or((end, end));
        if cut == end {
            // A hard cut: before what must stay whole, if that leaves something.
            let back = kept
                .iter()
                .copied()
                .chain(fences.iter().flat_map(|f| [(f.open, f.body), (f.close, f.end)]))
                .find(|&(a, b)| a < end && end < b && a > pos)
                .map_or(end, |(a, _)| a);
            cut = whole_char_cut(text, pos, back);
            next = cut;
        }
        let fence = fences.iter().find(|f| f.body <= cut && cut <= f.close);
        let mut part = String::from(prefix);
        match fence {
            Some(f) => {
                part.push_str(&text[pos..cut]);
                if !part.ends_with('\n') {
                    part.push('\n');
                }
                part.push_str(CLOSE);
                // A part that ends right at the closing fence has closed the block for good.
                if next >= f.close {
                    next = f.end;
                    reopen = None;
                } else {
                    reopen = Some(f.opener(text));
                }
            }
            None => {
                part.push_str(text[pos..cut].trim_end());
                reopen = None;
            }
        }
        if !part.trim().is_empty() {
            parts.push(part);
        }
        if next <= pos {
            next = pos + text[pos..].chars().next().map_or(1, char::len_utf8);
        }
        pos = next;
        if reopen.is_none() {
            pos = text.len() - text[pos..].trim_start().len();
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: &str = "aaaa0001-0000-4000-8000-000000000001";

    fn fits(parts: &[String], limit: usize) {
        for p in parts {
            assert!(utf16_len(p) <= limit, "{} > {limit}: {p:?}", utf16_len(p));
        }
    }

    #[test]
    fn a_text_that_fits_is_one_part() {
        assert_eq!(split_long("hello", 4096), ["hello"]);
        assert!(split_long("", 4096).is_empty());
        let exact = "a".repeat(4096);
        assert_eq!(split_long(&exact, 4096), [exact]);
    }

    #[test]
    fn parts_end_at_the_best_break_in_their_second_half() {
        // A blank line beats a line break and a sentence.
        let text = format!(
            "{}\n\n{}\n{}. {}",
            "a".repeat(30),
            "b".repeat(10),
            "c".repeat(5),
            "d".repeat(40)
        );
        let parts = split_long(&text, 64);
        fits(&parts, 64);
        assert_eq!(parts[0], "a".repeat(30));
        assert!(parts[1].starts_with('b'));
        // A line break in the first half is not taken: a sentence end later is.
        let text = format!("{}\n{}. {}", "a".repeat(5), "b".repeat(40), "c".repeat(40));
        let parts = split_long(&text, 64);
        assert_eq!(parts[0], format!("{}\n{}.", "a".repeat(5), "b".repeat(40)));
        assert_eq!(parts[1], "c".repeat(40));
    }

    #[test]
    fn words_stay_whole_and_a_long_word_is_cut() {
        let text = "word ".repeat(40);
        let parts = split_long(text.trim_end(), 64);
        fits(&parts, 64);
        assert!(parts.iter().all(|p| p.split(' ').all(|w| w == "word")));
        assert_eq!(parts.join(" "), text.trim_end());
        let long = "x".repeat(150);
        let parts = split_long(&long, 64);
        fits(&parts, 64);
        assert_eq!(parts.concat(), long);
    }

    #[test]
    fn the_limit_counts_utf16_and_keeps_emoji_whole() {
        // Each 👍🏽 is four UTF-16 units: a part of 64 holds sixteen of them.
        let text = "👍🏽".repeat(40);
        let parts = split_long(&text, 64);
        fits(&parts, 64);
        assert_eq!(parts.concat(), text);
        assert!(parts.iter().all(|p| p.chars().count() % 2 == 0));
    }

    /// A code block whose code starts with an empty line: the blank line right after the opening
    /// fence is no place to end a part, which would leave the fence without its block.
    #[test]
    fn a_blank_first_line_of_code_does_not_split_the_fence() {
        let text = format!("{}\n```\n\n{}\n```", "word ".repeat(600), "code\n".repeat(400));
        let parts = split_long(&text, 4096);
        fits(&parts, 4096);
        for p in &parts {
            assert_eq!(p.matches("```").count() % 2, 0, "{p:?}");
        }
    }

    /// Flags are pairs of letters: a run of them is cut between flags, into as few parts as fit.
    #[test]
    fn flags_are_cut_between_them() {
        let text = "🇷🇺".repeat(1500);
        let parts = split_long(&text, 4096);
        fits(&parts, 4096);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts.concat(), text);
        assert!(parts.iter().all(|p| p.chars().count() % 2 == 0));
        // The letters pair from the start of the run, whatever they spell: every part but the last
        // holds whole pairs.
        let text = format!("🇷{}", "🇷🇺".repeat(40));
        let parts = split_long(&text, 64);
        fits(&parts, 64);
        assert_eq!(parts.concat(), text);
        let (_, whole) = parts.split_last().unwrap();
        assert!(whole.iter().all(|p| p.chars().count() % 2 == 0), "{parts:?}");
    }

    /// A run of accents is not one character: it is cut, not followed back to the start.
    #[test]
    fn a_run_of_accents_does_not_shrink_the_parts() {
        let text = format!("a{}", "\u{0301}".repeat(5000));
        let parts = split_long(&text, 4096);
        fits(&parts, 4096);
        assert!(parts.len() <= 3, "{}", parts.len());
    }

    #[test]
    fn mentions_links_and_inline_code_are_not_cut() {
        let link = "[the plan](https://example.com/plan)";
        let text = format!("{} @{GUID} {link} `a b c` {}", "a ".repeat(10), "z ".repeat(30));
        // From a limit where each of them fits in a part with the room for a closing fence.
        for limit in 48..90 {
            let parts = split_long(&text, limit);
            fits(&parts, limit);
            let all = parts.join(" ");
            assert!(all.contains(&format!("@{GUID}")), "{limit}: {parts:?}");
            assert!(all.contains(link), "{limit}: {parts:?}");
            assert!(all.contains("`a b c`"), "{limit}: {parts:?}");
        }
    }

    #[test]
    fn a_code_block_cut_in_two_is_closed_and_opened_again() {
        let code: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
        let text = format!("Log:\n```rust\n{}\n```\nDone", code.join("\n"));
        let parts = split_long(&text, 80);
        fits(&parts, 80);
        assert!(parts.len() > 2);
        for p in &parts {
            assert_eq!(p.matches("```").count() % 2, 0, "{p:?}");
        }
        assert!(parts[1].starts_with("```rust\n"));
        // Every line of the code is in some part, whole.
        let blocks: Vec<String> = parts
            .iter()
            .flat_map(|p| crate::markup::parse_blocks(p))
            .filter_map(|b| match b {
                crate::markup::Block::Code(c) => Some(c),
                crate::markup::Block::Text(_) => None,
            })
            .collect();
        let lines: Vec<&str> = blocks.iter().flat_map(|b| b.lines()).filter(|l| *l != "rust").collect();
        assert_eq!(lines, code.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(parts.last().is_some_and(|p| p.ends_with("```\nDone")));
    }

    /// Mixed texts of every kind of piece: no panic, every part fits and has something, and nothing
    /// is lost.
    #[test]
    fn any_text_is_cut_into_parts_that_fit() {
        let pieces = [
            "word",
            " ",
            "\n",
            "\n\n",
            ". ",
            "```",
            "```rust\n",
            "\n```",
            "`x y`",
            "👍🏽",
            "é",
            "🇷🇺",
            "[a b](https://e.com/x)",
            "https://e.com/a-b",
            "@aaaa0001-0000-4000-8000-000000000001",
            "**",
            "ё",
            "\u{200D}",
            "😀",
        ];
        let mut seed: u64 = 7;
        let mut next = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        for _ in 0..400 {
            let text: String = (0..next() % 300).map(|_| pieces[next() % pieces.len()]).collect();
            let limit = 32 + next() % 120;
            let parts = split_long(&text, limit);
            fits(&parts, limit);
            assert!(parts.iter().all(|p| !p.trim().is_empty()), "{parts:?}");
            // Nothing is lost: what the text shows besides spaces and fences is all in the parts, in
            // order (a code block opened again repeats its language).
            let shown = |t: &str| {
                t.chars()
                    .filter(|c| !c.is_whitespace() && *c != '`')
                    .collect::<Vec<_>>()
            };
            let mut got = shown(&parts.concat()).into_iter();
            assert!(
                shown(&text).into_iter().all(|c| got.any(|g| g == c)),
                "{text:?} {parts:?}"
            );
        }
    }

    #[test]
    fn an_unclosed_fence_is_text() {
        let text = format!("```{}", "word ".repeat(30));
        let parts = split_long(&text, 64);
        fits(&parts, 64);
        assert!(parts.iter().skip(1).all(|p| !p.contains("```")));
    }
}
