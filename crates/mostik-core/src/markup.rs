//! Parsing of Telemost inline markup for display.
//!
//! Supports the same as the Messenger: `**bold**`, `__italic__`, `~~strike~~`,
//! `++underline++`, `` `code` ``, `[text](url)` links and bare `http(s)://…` links,
//! code blocks between ```` ``` ```` lines. Nesting is not supported.
//! Mentions are recognised by known names: `@First Last` for people, `@<name>` for public group chats
//! (a group mention, like user groups in Slack) and `@all` for everyone in the chat.

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
}

impl Mentionable<'_> {
    /// Mention at the start of `after` (the text right after `@`): the span and its length in bytes.
    /// The mention must end at a word boundary; the longest name wins, a person over a chat of the same name.
    fn find(&self, after: &str) -> Option<(Inline, usize)> {
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

/// Owned names behind a [`Mentionable`], for keeping them in state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MentionNames {
    pub people: Vec<String>,
    pub channels: Vec<String>,
}

impl MentionNames {
    /// Runs `f` with these names borrowed as a [`Mentionable`].
    pub fn with<R>(&self, f: impl FnOnce(&Mentionable) -> R) -> R {
        let people: Vec<&str> = self.people.iter().map(String::as_str).collect();
        let channels: Vec<&str> = self.channels.iter().map(String::as_str).collect();
        f(&Mentionable {
            people: &people,
            channels: &channels,
        })
    }
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

/// Text without markup, for previews in search, notifications and dialogs.
pub fn plain(text: &str) -> String {
    parse_inline(&text.replace("```", ""), &Mentionable::default())
        .into_iter()
        .map(|p| match p {
            Inline::Text(t)
            | Inline::Bold(t)
            | Inline::Italic(t)
            | Inline::Strike(t)
            | Inline::Underline(t)
            | Inline::Code(t)
            | Inline::Mention(t)
            | Inline::ChannelMention(t)
            | Inline::Link { text: t, .. } => t,
            Inline::MentionAll => format!("@{MENTION_ALL}"),
        })
        .collect()
}

/// Whether `text` mentions the user named `me_name`: by name, through a public chat they are in
/// (`who.channels` holds only such chats) or with `@all`. A mention inside bold, italic, strike,
/// underline or a link caption counts; code does not.
pub fn mentions_me(text: &str, who: &Mentionable, me_name: &str) -> bool {
    parse_blocks(text).iter().any(|b| match b {
        Block::Text(t) => inline_mentions(t, who, me_name),
        Block::Code(_) => false,
    })
}

fn inline_mentions(text: &str, who: &Mentionable, me_name: &str) -> bool {
    parse_inline(text, who).iter().any(|p| match p {
        Inline::Mention(n) => !me_name.is_empty() && n == me_name,
        Inline::ChannelMention(_) | Inline::MentionAll => true,
        Inline::Bold(t) | Inline::Italic(t) | Inline::Strike(t) | Inline::Underline(t) => {
            inline_mentions(t, who, me_name)
        }
        // A bare link is its own caption: parsing that again gives the same link, forever.
        Inline::Link { text, url } => text != url && inline_mentions(text, who, me_name),
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
    use super::Inline::*;
    use super::*;

    #[test]
    fn longer_words_are_not_mentions() {
        let channels = ["backend"];
        let who = Mentionable {
            people: &[],
            channels: &channels,
        };
        assert!(!mentions_me("встреча @all-hands в 15:00", &who, "Ivan"));
        assert!(!mentions_me("@all_devs", &who, "Ivan"));
        assert!(!mentions_me("@backend-team, @backend.team", &who, "Ivan"));
        assert!(mentions_me("всем @all.", &who, "Ivan"));
        assert!(mentions_me("@all, привет", &who, "Ivan"));
        assert!(mentions_me("в @backend: деплой", &who, "Ivan"));
        assert!(mentions_me("@backend's deploy", &who, "Ivan"));
    }

    #[test]
    fn mentions_inside_markup_count() {
        let names = ["Ivan Petrov"];
        let who = Mentionable {
            people: &names,
            channels: &[],
        };
        assert!(mentions_me("**@Ivan Petrov** please review", &who, "Ivan Petrov"));
        assert!(mentions_me("__@all__ release", &who, "Ivan Petrov"));
        assert!(mentions_me("[@all now](https://example.org)", &who, "Ivan Petrov"));
        // Bare links and links captioned with a link end, with or without a mention around.
        assert!(!mentions_me(
            "see https://example.org/x and [https://a.example](https://b.example)",
            &who,
            "Ivan Petrov"
        ));
        assert!(mentions_me("@all https://example.org", &who, "Ivan Petrov"));
        assert!(!mentions_me("`@all` in code", &who, "Ivan Petrov"));
        assert!(!mentions_me("```\n@all\n```", &who, "Ivan Petrov"));
    }

    const NONE: Mentionable = Mentionable {
        people: &[],
        channels: &[],
    };

    fn t(s: &str) -> Inline {
        Text(s.into())
    }

    fn people<'a>(names: &'a [&'a str]) -> Mentionable<'a> {
        Mentionable {
            people: names,
            channels: &[],
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
        assert_eq!(plain("hi @all"), "hi @all");
    }

    #[test]
    fn detects_own_mentions() {
        let who = Mentionable {
            people: &["Ivan", "Anna"],
            channels: &["backend"],
        };
        assert!(mentions_me("hi @Ivan", &who, "Ivan"));
        assert!(!mentions_me("hi @Anna", &who, "Ivan"));
        assert!(mentions_me("ping @backend", &who, "Ivan"));
        assert!(mentions_me("@all release", &who, "Ivan"));
        assert!(!mentions_me("ping @frontend", &who, "Ivan"));
        assert!(!mentions_me("```\n@all\n```", &who, "Ivan"));
        assert!(!mentions_me("@Ivan", &who, ""));
    }

    #[test]
    fn plain_strips_markers() {
        assert_eq!(plain("build **rc2**, `code` and ~~old~~"), "build rc2, code and old");
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
        assert_eq!(plain("[docs](https://e.org)"), "docs");
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
