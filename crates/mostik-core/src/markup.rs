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
    pub fn name_of(&self, id: &str) -> Option<&str> {
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
pub fn is_guid(s: &str) -> bool {
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

fn safe_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:"))
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// `[text](url)` at the start of the string: `(text, url, length in bytes)`.
fn md_link(s: &str) -> Option<(String, String, usize)> {
    let inner = s.strip_prefix('[')?;
    let close = inner.find("](")?;
    let text = &inner[..close];
    if text.is_empty() || text.contains('\n') || text.contains('[') {
        return None;
    }
    let tail = &inner[close + 2..];
    let end = tail.find(')')?;
    let url = &tail[..end];
    safe_url(url).then(|| (text.to_owned(), url.to_owned(), 1 + close + 2 + end + 1))
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
            let address = close.strip_prefix("](")?.strip_suffix(')')?.to_owned();
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
    md_link(&markup)
        .is_some_and(|(_, _, len)| len == markup.len())
        .then_some(markup)
}

/// Bare `http(s)://…` link up to whitespace; trailing punctuation is not part of the link.
fn bare_link(s: &str) -> Option<usize> {
    let lower: String = s.chars().take(8).collect::<String>().to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    let mut end = s
        .find(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '"')
        .unwrap_or(s.len());
    while end > 0 {
        let last = s[..end].chars().next_back()?;
        let unbalanced = last == ')' && s[..end].matches('(').count() < s[..end].matches(')').count();
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '»') || unbalanced {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    (end > "https://".len()).then_some(end)
}

type MakeInline = fn(String) -> Inline;

const PAIRS: &[(&str, MakeInline)] = &[
    ("`", Inline::Code),
    ("**", Inline::Bold),
    ("__", Inline::Italic),
    ("~~", Inline::Strike),
    ("++", Inline::Underline),
];

/// Splits a text block into inline spans; `who` is what can be mentioned.
pub fn parse_inline(text: &str, who: &Mentionable) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    'outer: while !rest.is_empty() {
        for (marker, make) in PAIRS {
            if let Some(after) = rest.strip_prefix(marker)
                && let Some(end) = after.find(marker)
                && end > 0
            {
                flush(&mut out, &mut plain);
                out.push(make(after[..end].to_owned()));
                rest = &after[end + marker.len()..];
                continue 'outer;
            }
        }
        if let Some((text, url, len)) = md_link(rest) {
            flush(&mut out, &mut plain);
            out.push(Inline::Link { text, url });
            rest = &rest[len..];
            continue;
        }
        let word_start = plain.chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        if word_start && let Some(len) = bare_link(rest) {
            flush(&mut out, &mut plain);
            out.push(Inline::Link {
                text: rest[..len].to_owned(),
                url: rest[..len].to_owned(),
            });
            rest = &rest[len..];
            continue;
        }
        if word_start
            && let Some(after) = rest.strip_prefix('@')
            && let Some((span, len)) = who.find(after)
        {
            flush(&mut out, &mut plain);
            out.push(span);
            rest = &after[len..];
            continue;
        }
        let ch = rest.chars().next().expect("rest is not empty");
        plain.push(ch);
        rest = &rest[ch.len_utf8()..];
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
}

const EDIT_PAIRS: &[(&str, Mark)] = &[
    ("`", Mark::Code),
    ("**", Mark::Bold),
    ("__", Mark::Italic),
    ("~~", Mark::Strike),
    ("++", Mark::Underline),
];

/// Splits the text of the message field into plain text and marked spans, the same markup as
/// [`parse_blocks`] and [`parse_inline`] read.
///
/// The markers of a code block take the line breaks a message drops around its code: after the
/// opening fence, before the closing one and after it, so the block shown in the field neither
/// starts nor ends with an empty line. A break that is all the block holds stays in it.
pub fn edit_spans(text: &str) -> Vec<EditSpan<'_>> {
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let mut found = None;
        if let Some(after) = rest.strip_prefix("```")
            && let Some(end) = after.find("```")
        {
            let body = &after[..end];
            let lead = usize::from(body.len() > 1 && body.starts_with('\n'));
            let trail = usize::from(body.len() - lead > 1 && body.ends_with('\n'));
            let next = usize::from(after[end + 3..].starts_with('\n'));
            found = Some((Mark::CodeBlock, 3 + lead, end - lead - trail, trail + 3 + next));
        }
        if found.is_none() {
            for (marker, kind) in EDIT_PAIRS {
                if let Some(after) = rest.strip_prefix(marker)
                    && let Some(end) = after.find(marker)
                    && end > 0
                {
                    found = Some((*kind, marker.len(), end, marker.len()));
                    break;
                }
            }
        }
        // Lengths in bytes of the opening marker, the text and the closing marker.
        let span = found.or_else(|| md_link(rest).map(|(t, _, len)| (Mark::Link, 1, t.len(), len - 1 - t.len())));
        let Some((kind, open, inner, close)) = span else {
            i += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        if plain_start < i {
            out.push(EditSpan::Plain(&text[plain_start..i]));
        }
        out.push(EditSpan::Marked {
            kind,
            open: &rest[..open],
            inner: &rest[open..open + inner],
            close: &rest[open + inner..open + inner + close],
        });
        i += open + inner + close;
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
