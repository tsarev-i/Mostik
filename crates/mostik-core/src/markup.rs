//! Parsing of Telemost inline markup for display.
//!
//! Supports the same as the Messenger: `**bold**`, `__italic__`, `~~strike~~`,
//! `++underline++`, `` `code` ``, `[text](url)` links and bare `http(s)://…` links,
//! code blocks between ```` ``` ```` lines. Nesting is not supported.
//! Mentions are recognised by known names: `@First Last`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Strike(String),
    Underline(String),
    Code(String),
    /// Name without the `@`.
    Mention(String),
    /// Link: caption and address (`http`, `https` or `mailto`).
    Link {
        text: String,
        url: String,
    },
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

/// Splits a text block into inline spans; `names` are display names that can be mentioned.
pub fn parse_inline(text: &str, names: &[&str]) -> Vec<Inline> {
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
        if let Some(after) = rest.strip_prefix('@')
            && let Some(name) = names
                .iter()
                .filter(|n| !n.is_empty() && after.starts_with(**n))
                .max_by_key(|n| n.len())
        {
            flush(&mut out, &mut plain);
            out.push(Inline::Mention((*name).to_owned()));
            rest = &after[name.len()..];
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
    parse_inline(&text.replace("```", ""), &[])
        .into_iter()
        .map(|p| match p {
            Inline::Text(t)
            | Inline::Bold(t)
            | Inline::Italic(t)
            | Inline::Strike(t)
            | Inline::Underline(t)
            | Inline::Code(t)
            | Inline::Mention(t)
            | Inline::Link { text: t, .. } => t,
        })
        .collect()
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

    fn t(s: &str) -> Inline {
        Text(s.into())
    }

    #[test]
    fn parses_styles() {
        assert_eq!(
            parse_inline("build **rc2**, `cargo test` and ~~old~~", &[]),
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
        assert_eq!(parse_inline("2 ** 3", &[]), vec![t("2 ** 3")]);
        assert_eq!(parse_inline("****", &[]), vec![t("****")]);
    }

    #[test]
    fn mentions_prefer_longest_name() {
        let names = ["Ivan", "Ivan Petrov"];
        assert_eq!(
            parse_inline("@Ivan Petrov hi", &names),
            vec![Mention("Ivan Petrov".into()), t(" hi")]
        );
        assert_eq!(parse_inline("mail a@b.ru", &names), vec![t("mail a@b.ru")]);
        // Non-ASCII names are matched on character boundaries.
        assert_eq!(parse_inline("@Иван!", &["Иван"]), vec![Mention("Иван".into()), t("!")]);
    }

    #[test]
    fn plain_strips_markers() {
        assert_eq!(plain("build **rc2**, `code` and ~~old~~"), "build rc2, code and old");
    }

    #[test]
    fn links() {
        assert_eq!(
            parse_inline("see https://example.com/a?b=1, and [docs](https://e.org/x)", &[]),
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
            parse_inline("[x](javascript:alert(1))", &[]),
            vec![t("[x](javascript:alert(1))")]
        );
        assert_eq!(
            parse_inline("(https://e.org/a)", &[]),
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
        assert_eq!(parse_inline("`**not bold**`", &[]), vec![Code("**not bold**".into())]);
    }
}
