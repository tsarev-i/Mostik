//! Parsing of Telemost inline markup for display.
//!
//! Supports the same as the Messenger: `**bold**`, `__italic__`, `~~strike~~`,
//! `++underline++`, `` `code` ``, `[text](url)` links and bare `http(s)://…` links,
//! code blocks between ```` ``` ```` lines. Nesting is not supported.
//! Mentions: `@<guid>` for people, as Telemost writes them (shown by name), and by known names:
//! `@First Last` for people (how older Mostik versions wrote them), `@<name>` for public group
//! chats (a group mention, like user groups in Slack) and `@all` for everyone in the chat.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Strike(String),
    Underline(String),
    Code(String),
    /// Person's name without the `@`.
    Mention(String),
    /// `@<guid>`: a person by their Telemost id, with the name if it is known.
    UserMention {
        id: String,
        name: Option<String>,
    },
    /// Public group chat name without the `@`: mentions its members.
    ChannelMention(String),
    /// `@all`: everyone in the chat.
    MentionAll,
    /// Link: caption and address (`http`, `https` or `mailto`).
    Link {
        text: String,
        url: String,
    },
}

/// The word after `@` that mentions everyone in the chat.
pub const MENTION_ALL: &str = "all";

/// What `@` can point at.
#[derive(Debug, Default, Clone, Copy)]
pub struct Mentionable<'a> {
    /// Display names of people.
    pub people: &'a [&'a str],
    /// Names of public group chats the user is a member of.
    pub channels: &'a [&'a str],
    /// Known people as `(id, display name)`, for `@<guid>`.
    pub ids: &'a [(&'a str, &'a str)],
}

impl Mentionable<'_> {
    /// Name of the person with Telemost id `id`, if known.
    fn name_of(&self, id: &str) -> Option<&str> {
        self.ids
            .iter()
            .find(|(i, n)| *i == id && !n.is_empty() && *n != id)
            .map(|(_, n)| *n)
    }

    /// Mention at the start of `after` (the text right after `@`): the span and its length in bytes.
    /// The mention must end at a word boundary; the longest name wins, a person over a chat of the same name.
    fn find(&self, after: &str) -> Option<(Inline, usize)> {
        if let Some(id) = guid_at(after) {
            let name = self.name_of(id).map(str::to_owned);
            return Some((
                Inline::UserMention {
                    id: id.to_owned(),
                    name,
                },
                id.len(),
            ));
        }
        let ends_word = |len: usize| ends_word(&after[len..]);
        let fits = |n: &&&str| !n.is_empty() && after.starts_with(**n) && ends_word(n.len());
        let person = self.people.iter().filter(fits).max_by_key(|n| n.len());
        let channel = self.channels.iter().filter(fits).max_by_key(|n| n.len());
        let best = match (person, channel) {
            (Some(p), Some(c)) if c.len() > p.len() => Some(Inline::ChannelMention((*c).to_owned())),
            (Some(p), _) => Some(Inline::Mention((*p).to_owned())),
            (None, Some(c)) => Some(Inline::ChannelMention((*c).to_owned())),
            (None, None) => None,
        };
        if let Some(span) = best {
            let len = match &span {
                Inline::Mention(n) | Inline::ChannelMention(n) => n.len(),
                _ => 0,
            };
            return Some((span, len));
        }
        let all = after.get(..MENTION_ALL.len())?;
        (all.eq_ignore_ascii_case(MENTION_ALL) && ends_word(MENTION_ALL.len()))
            .then_some((Inline::MentionAll, all.len()))
    }
}

/// Whether a mention ends where `rest` begins. Letters, digits, `-` and `_` go on with the word
/// (`@all-hands`, `@all_devs`); so does a `.` followed by one (`@backend.team`), while a full
/// stop at the end of a sentence (`@all.`) ends it, and so does `'` (`@Anna's`).
fn ends_word(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        None => true,
        Some(c) if c.is_alphanumeric() || c == '-' || c == '_' => false,
        Some('.') => chars.next().is_none_or(|c| !c.is_alphanumeric()),
        Some(_) => true,
    }
}

/// Length of a Telemost user id: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` in hex digits.
const GUID_LEN: usize = 36;

/// Whether `s` is a Telemost user id.
fn is_guid(s: &str) -> bool {
    s.len() == GUID_LEN
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// The user id at the start of `after` (the text right after `@`), ending at a word boundary.
fn guid_at(after: &str) -> Option<&str> {
    let id = after.get(..GUID_LEN)?;
    (is_guid(id) && ends_word(&after[GUID_LEN..])).then_some(id)
}

/// Owned names behind a [`Mentionable`], for keeping them in state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MentionNames {
    pub people: Vec<String>,
    pub channels: Vec<String>,
    /// Known people as `(id, display name)`.
    pub ids: Vec<(String, String)>,
}

impl MentionNames {
    /// Runs `f` with these names borrowed as a [`Mentionable`].
    pub fn with<R>(&self, f: impl FnOnce(&Mentionable) -> R) -> R {
        let people: Vec<&str> = self.people.iter().map(String::as_str).collect();
        let channels: Vec<&str> = self.channels.iter().map(String::as_str).collect();
        let ids: Vec<(&str, &str)> = self.ids.iter().map(|(i, n)| (i.as_str(), n.as_str())).collect();
        f(&Mentionable {
            people: &people,
            channels: &channels,
            ids: &ids,
        })
    }
}

/// Copies `text`, letting `f` replace what follows an `@` at the start of a word: `f` gets the
/// text after the `@` and returns the replacement and how many bytes it replaces. Code spans and
/// code blocks are copied as they are.
fn map_mentions(text: &str, mut f: impl FnMut(&str) -> Option<(String, usize)>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        let code = if let Some(after) = rest.strip_prefix("```") {
            after.find("```").map(|end| 3 + end + 3)
        } else if let Some(after) = rest.strip_prefix('`') {
            after.find('`').map(|end| 1 + end + 1)
        } else {
            None
        };
        if let Some(len) = code {
            out.push_str(&rest[..len]);
            rest = &rest[len..];
            continue;
        }
        let word_start = out.chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        if ch == '@'
            && word_start
            && let Some((with, len)) = f(&rest[1..])
        {
            out.push('@');
            out.push_str(&with);
            rest = &rest[1 + len..];
            continue;
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// Ids of the people mentioned as `@<guid>` outside code, each once: Telemost's `MentionedUserIds`.
pub fn mentioned_ids(text: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    map_mentions(text, |after| {
        if let Some(id) = guid_at(after)
            && !ids.iter().any(|i| i == id)
        {
            ids.push(id.to_owned());
        }
        None
    });
    ids
}

/// For editing: `@<guid>` of known people becomes `@Name`. Returns the text and the people it
/// named as `(name, id)`, so [`names_to_ids`] can turn them back.
pub fn ids_to_names(text: &str, who: &Mentionable) -> (String, Vec<(String, String)>) {
    let mut named = Vec::new();
    let out = map_mentions(text, |after| {
        let id = guid_at(after)?;
        let name = who.name_of(id)?;
        named.push((name.to_owned(), id.to_owned()));
        Some((name.to_owned(), id.len()))
    });
    (out, named)
}

/// For sending: `@Name` of a person in `known` (`(name, id)`) becomes `@<guid>`, the way Telemost
/// writes mentions. The longest name wins; the name must end at a word boundary.
pub fn names_to_ids(text: &str, known: &[(String, String)]) -> String {
    map_mentions(text, |after| {
        known
            .iter()
            .filter(|(name, _)| !name.is_empty() && after.starts_with(name.as_str()) && ends_word(&after[name.len()..]))
            .max_by_key(|(name, _)| name.len())
            .map(|(name, id)| (id.clone(), name.len()))
    })
}

/// Message block: plain text with inline markup, or a code block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Text(String),
    Code(String),
}

/// Splits text into blocks: everything between ```` ``` ```` is a code block (as in the Messenger and Slack).
/// An unclosed ```` ``` ```` stays text.
pub fn parse_blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let Some(end) = after.find("```") else { break };
        let before = rest[..start].trim_end_matches(['\n', ' ']);
        if !before.is_empty() {
            out.push(Block::Text(before.to_owned()));
        }
        let code = after[..end].strip_prefix('\n').unwrap_or(&after[..end]);
        out.push(Block::Code(code.strip_suffix('\n').unwrap_or(code).to_owned()));
        rest = after[end + 3..].trim_start_matches(['\n', ' ']);
    }
    if !rest.is_empty() {
        out.push(Block::Text(rest.to_owned()));
    }
    out
}

/// Whether a link from someone else's text may open: only `http`, `https` and `mailto`, so files
/// and other schemes do not, and nothing that hides whitespace or control characters.
pub fn safe_url(url: &str) -> bool {
    safe_scheme(url) && !url.chars().any(hides)
}

/// Whether `url` starts with `http://`, `https://` or `mailto:`, in any case.
fn safe_scheme(url: &str) -> bool {
    ["http://", "https://", "mailto:"].iter().any(|p| starts_ci(url, p))
}

/// Whether `s` starts with the ASCII `prefix`, in any case.
fn starts_ci(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

/// What a safe address must not have.
fn hides(c: char) -> bool {
    c.is_whitespace() || c.is_control()
}

/// A `[caption](address)` link in the text of the message field, in bytes of that text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    /// The whole markup, from `[` to `)`.
    pub range: Range<usize>,
    /// The caption between the brackets.
    pub caption: Range<usize>,
    pub address: String,
}

/// The link whose caption holds bytes `start..end` of `text`, edges included: the link the caret
/// or the selection is in. Links inside other markup are not found, as [`edit_spans`] shows them as text.
pub fn link_at(text: &str, start: usize, end: usize) -> Option<LinkSpan> {
    let mut at = 0;
    for span in edit_spans(text) {
        let EditSpan::Marked {
            kind,
            open,
            inner,
            close,
        } = span
        else {
            at += span_len(&span);
            continue;
        };
        let caption = at + open.len()..at + open.len() + inner.len();
        let range = at..caption.end + close.len();
        if kind == Mark::Link && caption.start <= start && end <= caption.end {
            let address = closing_address(close)?.to_owned();
            return Some(LinkSpan {
                range,
                caption,
                address,
            });
        }
        at = range.end;
    }
    None
}

/// The address of a link between its `(` and `)`, without the title after it.
fn link_target(between: &str) -> &str {
    between.split(' ').next().unwrap_or_default()
}

/// The address of a `[caption](address)` link from its closing part, `](address)` or
/// `](address "title")`, as [`edit_spans`] gives it.
pub fn closing_address(close: &str) -> Option<&str> {
    Some(link_target(close.strip_prefix("](")?.strip_suffix(')')?))
}

fn span_len(span: &EditSpan) -> usize {
    match span {
        EditSpan::Plain(t) => t.len(),
        EditSpan::Marked { open, inner, close, .. } => open.len() + inner.len() + close.len(),
    }
}

/// A link address typed by hand, made fit for `[caption](address)`: `https://` is added when it
/// has no scheme (`mailto:` for an e-mail address), and brackets and spaces are percent-encoded,
/// as the markup ends the address at the first `)`. `None` when it is empty or not a web or
/// mail address.
pub fn link_address(typed: &str) -> Option<String> {
    let typed = typed.trim();
    let lower = typed.to_ascii_lowercase();
    let full = if ["http://", "https://", "mailto:"].iter().any(|s| lower.starts_with(s)) {
        typed.to_owned()
    } else if has_scheme(typed) {
        return None;
    } else if typed.contains('@') && !typed.contains(['/', ':']) {
        format!("mailto:{typed}")
    } else {
        format!("https://{typed}")
    };
    let mut out = String::with_capacity(full.len());
    for c in full.chars() {
        if c == '(' || c == ')' || c.is_whitespace() || c.is_control() {
            let mut buf = [0; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    let bare = out.split_once(':').map_or("", |(_, rest)| rest.trim_start_matches('/'));
    (safe_url(&out) && !bare.is_empty()).then_some(out)
}

/// Whether `s` starts with a URL scheme (`ftp:`, `javascript:`), not a host and port (`localhost:8080`).
fn has_scheme(s: &str) -> bool {
    s.split_once(':').is_some_and(|(scheme, rest)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
            && !rest.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// An address from [`link_address`] as the user typed it: the brackets and spaces it encoded
/// shown as they are.
pub fn typed_address(address: &str) -> String {
    let mut out = String::with_capacity(address.len());
    let mut rest = address;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let code = rest.get(i + 1..i + 3).unwrap_or_default();
        match code.to_ascii_uppercase().as_str() {
            "28" => out.push('('),
            "29" => out.push(')'),
            "20" => out.push(' '),
            _ => {
                out.push('%');
                rest = &rest[i + 1..];
                continue;
            }
        }
        rest = &rest[i + 3..];
    }
    out.push_str(rest);
    out
}

/// Clipboard text that is one web or mail address (`http(s)://…`, `mailto:…`), as an address for
/// [`link_markup`].
pub fn pasted_link(text: &str) -> Option<String> {
    let text = text.trim();
    safe_url(text).then(|| link_address(text)).flatten()
}

/// `[caption](address)` for an address from [`link_address`]; `None` when `caption` cannot be a
/// link caption (empty, several lines, a `[`).
pub fn link_markup(caption: &str, address: &str) -> Option<String> {
    let markup = format!("[{caption}]({address})");
    Scan::new(&markup, false)
        .link_at(0)
        .is_some_and(|found| found.len() == markup.len())
        .then_some(markup)
}

type MakeInline = fn(String) -> Inline;

/// Markers that enclose inline markup, the same in a message and in its field: what each one is
/// in the field and the span it makes in a message.
const PAIRS: [(&str, Mark, MakeInline); 5] = [
    ("`", Mark::Code, Inline::Code),
    ("**", Mark::Bold, Inline::Bold),
    ("__", Mark::Italic, Inline::Italic),
    ("~~", Mark::Strike, Inline::Strike),
    ("++", Mark::Underline, Inline::Underline),
];

/// Markup found at a place of the text: its kind and the lengths in bytes of the opening marker,
/// the text and the closing marker.
#[derive(Debug, Clone, Copy)]
struct Found {
    kind: Mark,
    open: usize,
    inner: usize,
    close: usize,
}

impl Found {
    fn len(&self) -> usize {
        self.open + self.inner + self.close
    }
}

/// What [`Next`] looks for.
#[derive(Clone, Copy)]
enum Pattern {
    Str(&'static str),
    Char(fn(char) -> bool),
}

/// Where a pattern comes next in the text from some place on, remembered: asked again from a later
/// place that is not past it, the text is not searched again. Asked from places that only go
/// forward, as a scan does, it reads the text about once, where a `find` at every place read the
/// whole rest of it each time: seconds on a long run of unclosed markers.
struct Next {
    pattern: Pattern,
    /// Searched from the first place on, the pattern first comes at the second.
    memo: Option<(usize, Option<usize>)>,
}

impl Next {
    const fn new(pattern: Pattern) -> Self {
        Self { pattern, memo: None }
    }

    /// The first place of the pattern in `text` at byte `start` or after it.
    fn at(&mut self, text: &str, start: usize) -> Option<usize> {
        if let Some((from, found)) = self.memo
            && from <= start
            && found.is_none_or(|f| start <= f)
        {
            return found;
        }
        let rest = text.get(start..)?;
        let found = match self.pattern {
            Pattern::Str(s) => rest.find(s),
            Pattern::Char(f) => rest.find(f),
        }
        .map(|i| start + i);
        self.memo = Some((start, found));
        found
    }
}

/// The markup of a text, found from its start to its end the same way for a message
/// ([`parse_inline`]) and for its field ([`edit_spans`]).
struct Scan<'a> {
    text: &'a str,
    /// Code blocks are found too (the field): a bare address ends before one.
    blocks: bool,
    /// The closing marker of each of [`PAIRS`].
    pairs: [Next; PAIRS.len()],
    /// The `](` and the `)` that end the caption and the address of a `[caption](address)` link,
    /// and the `[` and the line break its caption must not have.
    link_mid: Next,
    link_end: Next,
    bracket: Next,
    line: Next,
    /// What a link's address must not have.
    hidden: Next,
    /// Where a bare address ends.
    url_end: Next,
    /// Code-block fences: the closing one of a block, and the next block for a bare address
    /// (a fence, then another after it).
    fence_close: Next,
    fence: Next,
    fence_after: Next,
}

impl<'a> Scan<'a> {
    fn new(text: &'a str, blocks: bool) -> Self {
        Self {
            text,
            blocks,
            pairs: PAIRS.map(|(marker, ..)| Next::new(Pattern::Str(marker))),
            link_mid: Next::new(Pattern::Str("](")),
            link_end: Next::new(Pattern::Str(")")),
            bracket: Next::new(Pattern::Str("[")),
            line: Next::new(Pattern::Str("\n")),
            hidden: Next::new(Pattern::Char(hides)),
            url_end: Next::new(Pattern::Char(|c| c.is_whitespace() || matches!(c, '<' | '>' | '"'))),
            fence_close: Next::new(Pattern::Str("```")),
            fence: Next::new(Pattern::Str("```")),
            fence_after: Next::new(Pattern::Str("```")),
        }
    }

    /// Inline markup at byte `i`, the first of: a pair of markers, a `[caption](address)` link, a
    /// bare `http(s)://…` address (only at the start of a word, `word_start`).
    fn inline_at(&mut self, i: usize, word_start: bool) -> Option<Found> {
        self.pair_at(i)
            .or_else(|| self.link_at(i))
            .or_else(|| word_start.then(|| self.url_at(i)).flatten())
    }

    /// Text between two markers of [`PAIRS`]: not empty, and the first marker that closes wins.
    fn pair_at(&mut self, i: usize) -> Option<Found> {
        let text = self.text;
        for ((marker, kind, _), close) in PAIRS.iter().zip(&mut self.pairs) {
            if !text[i..].starts_with(marker) {
                continue;
            }
            let start = i + marker.len();
            if let Some(end) = close.at(text, start)
                && end > start
            {
                return Some(Found {
                    kind: *kind,
                    open: marker.len(),
                    inner: end - start,
                    close: marker.len(),
                });
            }
        }
        None
    }

    /// `[caption](address)` or `[caption](address "title")`: a caption of one line without `[`, a
    /// safe address ([`safe_url`]).
    fn link_at(&mut self, i: usize) -> Option<Found> {
        let text = self.text;
        if !text[i..].starts_with('[') {
            return None;
        }
        let start = i + 1;
        let mid = self.link_mid.at(text, start)?;
        let before = |at: Option<usize>, end: usize| at.is_some_and(|a| a < end);
        if mid == start || before(self.bracket.at(text, start), mid) || before(self.line.at(text, start), mid) {
            return None;
        }
        let address = mid + 2;
        let end = self.link_end.at(text, address)?;
        if !safe_scheme(&text[address..end]) {
            return None;
        }
        // An address may be followed by a title, `(address "title")`, as the official client
        // writes a link pasted as it is: one space, then the title in quotes up to the `)`.
        if let Some(space) = self.hidden.at(text, address).filter(|&at| at < end) {
            let title = &text[space..end];
            let quoted =
                title.len() >= 3 && title.starts_with(' ') && title[1..].starts_with('"') && title.ends_with('"');
            if space == address || !quoted || before(self.line.at(text, space), end) {
                return None;
            }
        }
        Some(Found {
            kind: Mark::Link,
            open: 1,
            inner: mid - start,
            close: end + 1 - mid,
        })
    }

    /// A bare `http(s)://…` address up to whitespace (or a code block, in the field); punctuation
    /// at its end and a `)` that closes nothing in it are not part of it.
    fn url_at(&mut self, i: usize) -> Option<Found> {
        let text = self.text;
        let rest = &text[i..];
        if !starts_ci(rest, "http://") && !starts_ci(rest, "https://") {
            return None;
        }
        let mut stop = self.url_end.at(text, i).unwrap_or(text.len());
        if self.blocks
            && let Some(fence) = self.fence.at(text, i)
            && self.fence_after.at(text, fence + 3).is_some()
        {
            stop = stop.min(fence);
        }
        let url = &text[i..stop];
        // Counted once: the end only loses `)` and punctuation.
        let opened = url.matches('(').count();
        let mut closed = url.matches(')').count();
        let mut end = url.len();
        while let Some(last) = url[..end].chars().next_back() {
            let unbalanced = last == ')' && opened < closed;
            if !(matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '»') || unbalanced) {
                break;
            }
            end -= last.len_utf8();
            closed -= usize::from(last == ')');
        }
        (end > "https://".len()).then_some(Found {
            kind: Mark::Url,
            open: 0,
            inner: end,
            close: 0,
        })
    }

    /// A code block between ```` ``` ````. Its markers take the line breaks a message drops around
    /// its code: after the opening fence, before the closing one and after it. A break that is all
    /// the block holds stays in it.
    fn code_block_at(&mut self, i: usize) -> Option<Found> {
        let text = self.text;
        if !self.blocks || !text[i..].starts_with("```") {
            return None;
        }
        let end = self.fence_close.at(text, i + 3)?;
        let body = &text[i + 3..end];
        let lead = usize::from(body.len() > 1 && body.starts_with('\n'));
        let trail = usize::from(body.len() - lead > 1 && body.ends_with('\n'));
        let next = usize::from(text[end + 3..].starts_with('\n'));
        Some(Found {
            kind: Mark::CodeBlock,
            open: 3 + lead,
            inner: body.len() - lead - trail,
            close: trail + 3 + next,
        })
    }
}

/// Splits a text block into inline spans; `who` is what can be mentioned.
pub fn parse_inline(text: &str, who: &Mentionable) -> Vec<Inline> {
    let mut scan = Scan::new(text, false);
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let word_start = plain.chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        if let Some(found) = scan.inline_at(i, word_start) {
            flush(&mut out, &mut plain);
            let inner = text[i + found.open..i + found.open + found.inner].to_owned();
            out.push(match found.kind {
                Mark::Link => Inline::Link {
                    text: inner,
                    url: link_target(&text[i + found.open + found.inner + 2..i + found.len() - 1]).to_owned(),
                },
                Mark::Url => Inline::Link {
                    text: inner.clone(),
                    url: inner,
                },
                kind => match PAIRS.iter().find(|(_, k, _)| *k == kind) {
                    Some((_, _, make)) => make(inner),
                    None => Inline::Text(inner),
                },
            });
            i += found.len();
            continue;
        }
        if word_start
            && let Some(after) = rest.strip_prefix('@')
            && let Some((span, len)) = who.find(after)
        {
            flush(&mut out, &mut plain);
            out.push(span);
            i += 1 + len;
            continue;
        }
        let ch = rest.chars().next().expect("rest is not empty");
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut out, &mut plain);
    out
}

/// A piece of the message field's text, for showing markup as it will look while keeping the
/// text as is: the pieces put together give the text back exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditSpan<'a> {
    Plain(&'a str),
    /// Text between markers; `kind` says which markup.
    Marked {
        kind: Mark,
        open: &'a str,
        inner: &'a str,
        close: &'a str,
    },
}

/// Markup kinds of [`EditSpan::Marked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Strike,
    Underline,
    Code,
    CodeBlock,
    Link,
    /// A bare `http(s)://…` address, which a message shows as a link: it has no markers.
    Url,
}

/// Splits the text of the message field into plain text and marked spans, the same markup as
/// [`parse_blocks`] and [`parse_inline`] read, bare web addresses included.
///
/// The markers of a code block take the line breaks a message drops around its code: after the
/// opening fence, before the closing one and after it, so the block shown in the field neither
/// starts nor ends with an empty line. A break that is all the block holds stays in it.
pub fn edit_spans(text: &str) -> Vec<EditSpan<'_>> {
    let mut scan = Scan::new(text, true);
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    while i < text.len() {
        let word_start = text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let Some(found) = scan.code_block_at(i).or_else(|| scan.inline_at(i, word_start)) else {
            i += text[i..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        if plain_start < i {
            out.push(EditSpan::Plain(&text[plain_start..i]));
        }
        let (inner, close) = (i + found.open, i + found.open + found.inner);
        out.push(EditSpan::Marked {
            kind: found.kind,
            open: &text[i..inner],
            inner: &text[inner..close],
            close: &text[close..i + found.len()],
        });
        i += found.len();
        plain_start = i;
    }
    if plain_start < text.len() {
        out.push(EditSpan::Plain(&text[plain_start..]));
    }
    out
}

/// Text without markup, for previews in search, notifications and dialogs; `who` names the
/// people mentioned as `@<guid>`.
pub fn plain(text: &str, who: &Mentionable) -> String {
    // Names are left as they are written: only ids need replacing.
    let who = Mentionable {
        people: &[],
        channels: &[],
        ..*who
    };
    plain_inline(&text.replace("```", ""), &who)
}

fn plain_inline(text: &str, who: &Mentionable) -> String {
    parse_inline(text, who)
        .into_iter()
        .map(|p| match p {
            Inline::Bold(t) | Inline::Italic(t) | Inline::Strike(t) | Inline::Underline(t) => plain_inline(&t, who),
            // A bare link is its own caption: parsing that again gives the same link, forever.
            Inline::Link { text, url } if text != url => plain_inline(&text, who),
            Inline::Text(t)
            | Inline::Code(t)
            | Inline::Mention(t)
            | Inline::ChannelMention(t)
            | Inline::Link { text: t, .. } => t,
            Inline::UserMention { name, .. } => format!("@{}", name.as_deref().unwrap_or(crate::UNKNOWN_USER)),
            Inline::MentionAll => format!("@{MENTION_ALL}"),
        })
        .collect()
}

/// The user a mention may point at: their Telemost id and display name.
#[derive(Debug, Clone, Copy)]
pub struct Me<'a> {
    pub id: &'a str,
    pub name: &'a str,
}

/// Whether `text` mentions the user `me`: by id, by name, through a public chat they are in
/// (`who.channels` holds only such chats) or with `@all`. A mention inside bold, italic, strike,
/// underline or a link caption counts; code does not.
pub fn mentions_me(text: &str, who: &Mentionable, me: Me) -> bool {
    parse_blocks(text).iter().any(|b| match b {
        Block::Text(t) => inline_mentions(t, who, me),
        Block::Code(_) => false,
    })
}

fn inline_mentions(text: &str, who: &Mentionable, me: Me) -> bool {
    parse_inline(text, who).iter().any(|p| match p {
        Inline::Mention(n) => !me.name.is_empty() && n == me.name,
        Inline::UserMention { id, .. } => !me.id.is_empty() && id == me.id,
        Inline::ChannelMention(_) | Inline::MentionAll => true,
        Inline::Bold(t) | Inline::Italic(t) | Inline::Strike(t) | Inline::Underline(t) => inline_mentions(t, who, me),
        // A bare link is its own caption: parsing that again gives the same link, forever.
        Inline::Link { text, url } => text != url && inline_mentions(text, who, me),
        Inline::Text(_) | Inline::Code(_) => false,
    })
}

fn flush(out: &mut Vec<Inline>, plain: &mut String) {
    if !plain.is_empty() {
        out.push(Inline::Text(std::mem::take(plain)));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn edit_spans_give_the_text_back() {
        for text in [
            "a **b** c __d__ ~~e~~ ++f++ `g` [h](https://x.y) i",
            "```\nfn x() {}\n``` after",
            "a\n```\nx\n\n```\nb ```\n``` ``````",
            "**** ** unclosed __x",
            "Привет, **мир** 👋",
            "link https://x.y/a(b) and https://z.ru.",
            "",
        ] {
            let joined: String = edit_spans(text)
                .iter()
                .map(|s| match s {
                    EditSpan::Plain(t) => t.to_string(),
                    EditSpan::Marked { open, inner, close, .. } => format!("{open}{inner}{close}"),
                })
                .collect();
            assert_eq!(joined, text);
        }
    }

    #[test]
    fn unclosed_markers_take_linear_time() {
        // Each of these took seconds when every place searched the rest of the text again (#18);
        // now all of them together take milliseconds. The limit leaves room for a slow machine.
        let n = 50_000;
        let texts = [
            format!("https://x.ru/{}", ")".repeat(n)),
            "[".repeat(n),
            "[a ".repeat(n),
            "**a ".repeat(n),
            "` a".repeat(n),
            "[a](".repeat(n),
            format!("{}x)", "[a](http://".repeat(n)),
            format!("{}```", "h ".repeat(n)),
            "[a\n".repeat(n),
        ];
        let started = std::time::Instant::now();
        for text in &texts {
            parse_inline(text, &Mentionable::default());
            edit_spans(text);
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn edit_spans_find_markup() {
        let spans = edit_spans("a **b** [c](https://x.y)");
        assert_eq!(spans[0], EditSpan::Plain("a "));
        assert_eq!(
            spans[1],
            EditSpan::Marked {
                kind: Mark::Bold,
                open: "**",
                inner: "b",
                close: "**"
            }
        );
        assert_eq!(
            spans[3],
            EditSpan::Marked {
                kind: Mark::Link,
                open: "[",
                inner: "c",
                close: "](https://x.y)"
            }
        );
        assert_eq!(edit_spans("****"), vec![EditSpan::Plain("****")]);
    }

    #[test]
    fn edit_spans_find_bare_links() {
        let url = |inner| EditSpan::Marked {
            kind: Mark::Url,
            open: "",
            inner,
            close: "",
        };
        assert_eq!(
            edit_spans("see https://x.y/a?b=1, ok"),
            vec![
                EditSpan::Plain("see "),
                url("https://x.y/a?b=1"),
                EditSpan::Plain(", ok")
            ]
        );
        assert_eq!(edit_spans("HTTP://x.y"), vec![url("HTTP://x.y")]);
        // As in a message: not inside a word, code or a link caption, and not a bare scheme.
        assert_eq!(edit_spans("ahttps://x.y"), vec![EditSpan::Plain("ahttps://x.y")]);
        assert_eq!(edit_spans("https://"), vec![EditSpan::Plain("https://")]);
        // A code block right after it is a block, as in a message; an unclosed fence is text.
        assert!(matches!(
            edit_spans("https://x.ru/```a```")[..],
            [
                EditSpan::Marked {
                    kind: Mark::Url,
                    inner: "https://x.ru/",
                    ..
                },
                EditSpan::Marked {
                    kind: Mark::CodeBlock,
                    ..
                }
            ]
        ));
        assert_eq!(edit_spans("https://x.ru/```a"), vec![url("https://x.ru/```a")]);
        assert!(matches!(
            edit_spans("`https://x.y`")[..],
            [EditSpan::Marked { kind: Mark::Code, .. }]
        ));
        assert!(matches!(
            edit_spans("[https://x.y](https://x.y)")[..],
            [EditSpan::Marked { kind: Mark::Link, .. }]
        ));
        assert!(matches!(
            edit_spans("**b**https://x.y")[..],
            [
                EditSpan::Marked { kind: Mark::Bold, .. },
                EditSpan::Marked { kind: Mark::Url, .. }
            ]
        ));
    }

    #[test]
    fn code_block_markers_take_its_line_breaks() {
        let block = |open, inner, close| EditSpan::Marked {
            kind: Mark::CodeBlock,
            open,
            inner,
            close,
        };
        assert_eq!(
            edit_spans("a\n```\nlet x;\n```\nb"),
            vec![
                EditSpan::Plain("a\n"),
                block("```\n", "let x;", "\n```\n"),
                EditSpan::Plain("b")
            ]
        );
        // An empty line typed at the end of the code stays in it.
        assert_eq!(edit_spans("```\nx\n\n```"), vec![block("```\n", "x\n", "\n```")]);
        // A block of one empty line (the button with nothing selected) keeps it for the caret.
        assert_eq!(edit_spans("```\n\n```"), vec![block("```\n", "\n", "```")]);
        assert_eq!(edit_spans("```\n```"), vec![block("```", "\n", "```")]);
        assert_eq!(
            edit_spans("```a``` b"),
            vec![block("```", "a", "```"), EditSpan::Plain(" b")]
        );
    }

    #[test]
    fn finds_the_link_around_the_caret() {
        let text = "see [the docs](https://e.org/a%28b%29) now";
        let link = link_at(text, 9, 9).unwrap();
        assert_eq!(&text[link.range.clone()], "[the docs](https://e.org/a%28b%29)");
        assert_eq!(&text[link.caption.clone()], "the docs");
        assert_eq!(link.address, "https://e.org/a%28b%29");
        // The edges of the caption count, the markers and the text around do not.
        assert_eq!(link_at(text, 5, 13).map(|l| l.caption), Some(5..13));
        assert_eq!(link_at(text, 4, 4), None);
        assert_eq!(link_at(text, 2, 9), None);
        assert_eq!(link_at("**[a](https://e.org)**", 4, 4), None);
    }

    #[test]
    fn link_addresses() {
        assert_eq!(
            link_address(" example.com/x ").as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(link_address("HTTP://E.org").as_deref(), Some("HTTP://E.org"));
        assert_eq!(
            link_address("https://en.wikipedia.org/wiki/Rust_(language) x").as_deref(),
            Some("https://en.wikipedia.org/wiki/Rust_%28language%29%20x")
        );
        assert_eq!(
            link_address("anna.smirnova@example.com").as_deref(),
            Some("mailto:anna.smirnova@example.com")
        );
        assert_eq!(
            link_address("localhost:8080/a").as_deref(),
            Some("https://localhost:8080/a")
        );
        assert_eq!(link_address(""), None);
        assert_eq!(link_address("https://"), None);
        assert_eq!(link_address("ftp://e.org"), None);
        assert_eq!(link_address("javascript:alert(1)"), None);
        let typed = "https://e.org/a (b)%2F";
        assert_eq!(typed_address(&link_address(typed).unwrap()), typed);
        assert_eq!(typed_address("https://e.org/%2"), "https://e.org/%2");
        // The markup keeps an encoded address whole.
        let markup = link_markup("Rust", &link_address("https://e.org/a_(b)").unwrap()).unwrap();
        assert_eq!(
            parse_inline(&markup, &NONE),
            vec![Link {
                text: "Rust".into(),
                url: "https://e.org/a_%28b%29".into()
            }]
        );
        assert_eq!(link_markup("a [b]", "https://e.org"), None);
        assert_eq!(link_markup("two\nlines", "https://e.org"), None);
        assert_eq!(link_markup("", "https://e.org"), None);
        assert_eq!(
            pasted_link(" https://e.org/(x)\n").as_deref(),
            Some("https://e.org/%28x%29")
        );
        assert_eq!(
            pasted_link("mailto:olga@example.com").as_deref(),
            Some("mailto:olga@example.com")
        );
        assert_eq!(pasted_link("e.org"), None);
        assert_eq!(pasted_link("https://e.org and more"), None);
    }

    #[test]
    fn only_web_and_mail_links_are_safe() {
        for ok in ["https://e.org/a?b=1", "HTTP://E.ORG", "mailto:olga@example.com"] {
            assert!(safe_url(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://e.org",
            "e.org",
            "https://e.org/a b",
            "https://e.org/\u{0}",
            " https://e.org",
        ] {
            assert!(!safe_url(bad), "{bad}");
        }
    }

    use super::Inline::*;
    use super::*;

    #[test]
    fn longer_words_are_not_mentions() {
        let channels = ["backend"];
        let who = Mentionable {
            people: &[],
            channels: &channels,
            ids: &[],
        };
        assert!(!mentions_me("встреча @all-hands в 15:00", &who, me("Ivan")));
        assert!(!mentions_me("@all_devs", &who, me("Ivan")));
        assert!(!mentions_me("@backend-team, @backend.team", &who, me("Ivan")));
        assert!(mentions_me("всем @all.", &who, me("Ivan")));
        assert!(mentions_me("@all, привет", &who, me("Ivan")));
        assert!(mentions_me("в @backend: деплой", &who, me("Ivan")));
        assert!(mentions_me("@backend's deploy", &who, me("Ivan")));
    }

    #[test]
    fn mentions_inside_markup_count() {
        let names = ["Ivan Petrov"];
        let who = Mentionable {
            people: &names,
            channels: &[],
            ids: &[],
        };
        assert!(mentions_me("**@Ivan Petrov** please review", &who, me("Ivan Petrov")));
        assert!(mentions_me("__@all__ release", &who, me("Ivan Petrov")));
        assert!(mentions_me("[@all now](https://example.org)", &who, me("Ivan Petrov")));
        // Bare links and links captioned with a link end, with or without a mention around.
        assert!(!mentions_me(
            "see https://example.org/x and [https://a.example](https://b.example)",
            &who,
            me("Ivan Petrov")
        ));
        assert!(mentions_me("@all https://example.org", &who, me("Ivan Petrov")));
        assert!(!mentions_me("`@all` in code", &who, me("Ivan Petrov")));
        assert!(!mentions_me("```\n@all\n```", &who, me("Ivan Petrov")));
    }

    const NONE: Mentionable = Mentionable {
        people: &[],
        channels: &[],
        ids: &[],
    };

    fn me(name: &str) -> Me<'_> {
        Me { id: "", name }
    }

    fn t(s: &str) -> Inline {
        Text(s.into())
    }

    fn people<'a>(names: &'a [&'a str]) -> Mentionable<'a> {
        Mentionable {
            people: names,
            channels: &[],
            ids: &[],
        }
    }

    #[test]
    fn parses_styles() {
        assert_eq!(
            parse_inline("build **rc2**, `cargo test` and ~~old~~", &NONE),
            vec![
                t("build "),
                Bold("rc2".into()),
                t(", "),
                Code("cargo test".into()),
                t(" and "),
                Strike("old".into())
            ]
        );
    }

    #[test]
    fn unclosed_marker_is_text() {
        assert_eq!(parse_inline("2 ** 3", &NONE), vec![t("2 ** 3")]);
        assert_eq!(parse_inline("****", &NONE), vec![t("****")]);
    }

    #[test]
    fn mentions_prefer_longest_name() {
        let names = ["Ivan", "Ivan Petrov"];
        assert_eq!(
            parse_inline("@Ivan Petrov hi", &people(&names)),
            vec![Mention("Ivan Petrov".into()), t(" hi")]
        );
        assert_eq!(parse_inline("mail a@b.ru", &people(&names)), vec![t("mail a@b.ru")]);
        // Non-ASCII names are matched on character boundaries.
        assert_eq!(
            parse_inline("@Иван!", &people(&["Иван"])),
            vec![Mention("Иван".into()), t("!")]
        );
        // The name must end at a word boundary.
        assert_eq!(parse_inline("@Ivanov", &people(&names)), vec![t("@Ivanov")]);
        assert_eq!(
            parse_inline("@Ivan Petrovich", &people(&names)),
            vec![Mention("Ivan".into()), t(" Petrovich")]
        );
    }

    #[test]
    fn mentions_channels_and_all() {
        let who = Mentionable {
            people: &["Ivan"],
            channels: &["backend", "Ivan team"],
            ids: &[],
        };
        assert_eq!(
            parse_inline("@backend, @Ivan team and @ALL!", &who),
            vec![
                ChannelMention("backend".into()),
                t(", "),
                ChannelMention("Ivan team".into()),
                t(" and "),
                MentionAll,
                t("!")
            ]
        );
        assert_eq!(
            parse_inline("@backends @allow a@all", &who),
            vec![t("@backends @allow a@all")]
        );
        assert_eq!(plain("hi @all", &NONE), "hi @all");
    }

    #[test]
    fn detects_own_mentions() {
        let who = Mentionable {
            people: &["Ivan", "Anna"],
            channels: &["backend"],
            ids: &[],
        };
        assert!(mentions_me("hi @Ivan", &who, me("Ivan")));
        assert!(!mentions_me("hi @Anna", &who, me("Ivan")));
        assert!(mentions_me("ping @backend", &who, me("Ivan")));
        assert!(mentions_me("@all release", &who, me("Ivan")));
        assert!(!mentions_me("ping @frontend", &who, me("Ivan")));
        assert!(!mentions_me("```\n@all\n```", &who, me("Ivan")));
        assert!(!mentions_me("@Ivan", &who, me("")));
    }

    const ANNA: &str = "aaaa0001-0000-4000-8000-000000000001";
    const IVAN: &str = "bbbb0002-0000-4000-8000-000000000002";

    #[test]
    fn mentions_by_id() {
        let ids = [(ANNA, "Anna Smirnova")];
        let who = Mentionable {
            people: &["Anna Smirnova"],
            channels: &[],
            ids: &ids,
        };
        assert_eq!(
            parse_inline(&format!("@{ANNA} hi, CC @{IVAN}"), &who),
            vec![
                UserMention {
                    id: ANNA.into(),
                    name: Some("Anna Smirnova".into())
                },
                t(" hi, CC "),
                UserMention {
                    id: IVAN.into(),
                    name: None
                },
            ]
        );
        // Not an id: too short, longer, or not at the start of a word.
        assert_eq!(
            parse_inline(&format!("@{}", &ANNA[..35]), &who),
            vec![t(&format!("@{}", &ANNA[..35]))]
        );
        assert_eq!(parse_inline(&format!("@{ANNA}0"), &who), vec![t(&format!("@{ANNA}0"))]);
        assert_eq!(parse_inline(&format!("a@{ANNA}"), &who), vec![t(&format!("a@{ANNA}"))]);
        assert_eq!(
            plain(&format!("**@{ANNA}** and @{IVAN}"), &who),
            "@Anna Smirnova and @Unknown user"
        );
        let me_ivan = Me { id: IVAN, name: "Ivan" };
        assert!(mentions_me(&format!("CC @{IVAN}, look"), &who, me_ivan));
        assert!(!mentions_me(&format!("CC @{ANNA}"), &who, me_ivan));
        assert!(!mentions_me(&format!("`@{IVAN}`"), &who, me_ivan));
    }

    #[test]
    fn converts_mentions_for_sending_and_editing() {
        let known = vec![
            ("Anna".to_owned(), IVAN.to_owned()),
            ("Anna Smirnova".to_owned(), ANNA.to_owned()),
        ];
        assert_eq!(
            names_to_ids("@Anna Smirnova, @Anna and @all; `@Anna` a@Anna @Annabel", &known),
            format!("@{ANNA}, @{IVAN} and @all; `@Anna` a@Anna @Annabel")
        );
        assert_eq!(
            names_to_ids("```\n@Anna\n``` @Anna", &known),
            format!("```\n@Anna\n``` @{IVAN}")
        );
        let text = format!("hi @{ANNA} and @{IVAN} `@{ANNA}` @{ANNA}");
        assert_eq!(mentioned_ids(&text), vec![ANNA.to_owned(), IVAN.to_owned()]);
        let ids = [(ANNA, "Anna Smirnova")];
        let who = Mentionable { ids: &ids, ..NONE };
        let (edit, named) = ids_to_names(&text, &who);
        assert_eq!(edit, format!("hi @Anna Smirnova and @{IVAN} `@{ANNA}` @Anna Smirnova"));
        assert_eq!(named[0], ("Anna Smirnova".to_owned(), ANNA.to_owned()));
        assert_eq!(names_to_ids(&edit, &named), text);
    }

    #[test]
    fn plain_strips_markers() {
        assert_eq!(
            plain("build **rc2**, `code` and ~~old~~", &NONE),
            "build rc2, code and old"
        );
    }

    #[test]
    fn links() {
        assert_eq!(
            parse_inline("see https://example.com/a?b=1, and [docs](https://e.org/x)", &NONE),
            vec![
                t("see "),
                Link {
                    text: "https://example.com/a?b=1".into(),
                    url: "https://example.com/a?b=1".into()
                },
                t(", and "),
                Link {
                    text: "docs".into(),
                    url: "https://e.org/x".into()
                },
            ]
        );
        assert_eq!(
            parse_inline("[x](javascript:alert(1))", &NONE),
            vec![t("[x](javascript:alert(1))")]
        );
        assert_eq!(
            parse_inline("(https://e.org/a)", &NONE),
            vec![
                t("("),
                Link {
                    text: "https://e.org/a".into(),
                    url: "https://e.org/a".into()
                },
                t(")")
            ]
        );
        assert_eq!(plain("[docs](https://e.org)", &NONE), "docs");
    }

    #[test]
    fn a_link_may_have_a_title() {
        let text = "Ингрес [host.ru](https://host.ru:443 \"mssngr:raw-link:v1:null\"), порт 443";
        assert_eq!(
            parse_inline(text, &NONE),
            vec![
                t("Ингрес "),
                Link {
                    text: "host.ru".into(),
                    url: "https://host.ru:443".into()
                },
                t(", порт 443"),
            ]
        );
        let start = text.find("host").unwrap();
        let link = link_at(text, start, start).unwrap();
        assert_eq!(link.address, "https://host.ru:443");
        assert_eq!(
            &text[link.range],
            "[host.ru](https://host.ru:443 \"mssngr:raw-link:v1:null\")"
        );
        // Only one space and a title in quotes: anything else after the address is no link with a
        // caption (a bare address inside it still is one).
        for bad in [
            "[a](https://x.ru  \"t\")",
            "[a](https://x.ru t)",
            "[a](https://x.ru \"t)",
            "[a](https://x.ru \"t\nu\")",
            "[a]( \"t\")",
            "[a](javascript:x \"t\")",
        ] {
            let captioned = |i: &Inline| matches!(i, Link { text, .. } if text == "a");
            assert!(!parse_inline(bad, &NONE).iter().any(captioned), "{bad:?}");
        }
    }

    #[test]
    fn code_blocks() {
        assert_eq!(
            parse_blocks("before\n```\nlet x = 1;\n```\nafter"),
            vec![
                Block::Text("before".into()),
                Block::Code("let x = 1;".into()),
                Block::Text("after".into())
            ]
        );
        assert_eq!(parse_blocks("```a```"), vec![Block::Code("a".into())]);
        assert_eq!(parse_blocks("no ``` end"), vec![Block::Text("no ``` end".into())]);
    }

    #[test]
    fn code_keeps_markers_inside() {
        assert_eq!(parse_inline("`**not bold**`", &NONE), vec![Code("**not bold**".into())]);
    }
}
