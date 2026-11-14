// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! URL-like tokens, and the rows of a line that mixes them with prose: a
//! URL token is never split, a prose word moves whole and is split only
//! when it is wider than a row.
//!
//! URL detection is heuristic. False positives suppress hyphenation for
//! that line; false negatives let a URL get split. File paths like
//! `src/main.rs` are not matched.

use std::ops::Range;

use textwrap::WordSeparator;
use unicode_segmentation::UnicodeSegmentation;

use super::display_width;

/// Whether `text` has a URL-like token, and a substantive non-URL one
/// (decorative markers — `-`, `1.`, `|`, `│` — are not substantive).
///
/// Recognized URLs:
/// - Absolute URLs with a scheme (`https://…`, `ftp://…`, custom `myapp://…`).
/// - Bare domain URLs (`example.com/path`, `www.example.com`, `localhost:3000/api`).
/// - IPv4 hosts with a path (`192.168.1.1:8080/health`).
///
/// Surrounding punctuation (`()[]{}< >,.;:!'"`) is stripped before
/// checking. Tokens that look like file paths (`src/main.rs`, `foo/bar`)
/// are intentionally rejected — the host portion must be a valid domain
/// name (with a recognized TLD), an IPv4 address, or `localhost`.
pub(super) fn url_and_prose_tokens(text: &str) -> (bool, bool) {
    let mut saw_url = false;
    let mut saw_non_url = false;

    for raw_token in text.split_ascii_whitespace() {
        if is_url_like_token(raw_token) {
            saw_url = true;
        } else if is_substantive_non_url_token(raw_token) {
            saw_non_url = true;
        }

        if saw_url && saw_non_url {
            break;
        }
    }

    (saw_url, saw_non_url)
}

/// Decides whether a single whitespace-delimited token is URL-like.
///
/// Strips surrounding punctuation, then checks for an absolute URL
/// (with `://`) or a bare domain URL (recognized host + path/query/fragment).
fn is_url_like_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    !token.is_empty() && (is_absolute_url_like(token) || is_bare_url_like(token))
}

fn is_substantive_non_url_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    if token.is_empty() || is_decorative_marker_token(raw_token, token) {
        return false;
    }

    token.chars().any(char::is_alphanumeric)
}

fn is_decorative_marker_token(raw_token: &str, token: &str) -> bool {
    let raw = raw_token.trim();
    matches!(
        raw,
        "-" | "*"
            | "+"
            | "•"
            | "◦"
            | "▪"
            | ">"
            | "|"
            | "│"
            | "┆"
            | "└"
            | "├"
            | "┌"
            | "┐"
            | "┘"
            | "┼"
    ) || is_ordered_list_marker(raw, token)
}

fn is_ordered_list_marker(raw_token: &str, token: &str) -> bool {
    token.chars().all(|c| c.is_ascii_digit())
        && (raw_token.ends_with('.') || raw_token.ends_with(')'))
}

fn trim_url_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ','
                | '.'
                | ';'
                | ':'
                | '!'
                | '\''
                | '"'
        )
    })
}

/// Checks for `scheme://host` patterns. Uses `url::Url::parse` for
/// well-known schemes; falls back to `has_valid_scheme_prefix` for
/// custom schemes that the `url` crate rejects.
fn is_absolute_url_like(token: &str) -> bool {
    if !token.contains("://") {
        return false;
    }

    if let Ok(url) = url::Url::parse(token) {
        let scheme = url.scheme().to_ascii_lowercase();
        if matches!(
            scheme.as_str(),
            "http" | "https" | "ftp" | "ftps" | "ws" | "wss"
        ) {
            return url.host_str().is_some();
        }
        return true;
    }

    has_valid_scheme_prefix(token)
}

fn has_valid_scheme_prefix(token: &str) -> bool {
    let Some((scheme, rest)) = token.split_once("://") else {
        return false;
    };
    if scheme.is_empty() || rest.is_empty() {
        return false;
    }

    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

/// Checks for bare-domain URLs without a scheme: `host[:port]/path`,
/// `host[:port]?query`, or `host[:port]#fragment`.
///
/// Requires that the host is `localhost`, an IPv4 address, or a valid
/// domain name. Bare `host.tld` without a path/query/fragment is only
/// accepted when the host starts with `www.`.
///
/// IPv6 bracket notation (`[::1]:8080`) is intentionally not handled.
fn is_bare_url_like(token: &str) -> bool {
    let (host_port, has_trailer) = split_host_port_and_trailer(token);
    if host_port.is_empty() {
        return false;
    }

    // Require URL-ish trailer for bare hosts unless token starts with www.
    if !has_trailer && !host_port.to_ascii_lowercase().starts_with("www.") {
        return false;
    }

    let (host, port) = split_host_and_port(host_port);
    if host.is_empty() {
        return false;
    }
    if let Some(port) = port {
        if !is_valid_port(port) {
            return false;
        }
    }

    host.eq_ignore_ascii_case("localhost") || is_ipv4(host) || is_domain_name(host)
}

fn split_host_port_and_trailer(token: &str) -> (&str, bool) {
    if let Some(idx) = token.find(['/', '?', '#']) {
        (&token[..idx], true)
    } else {
        (token, false)
    }
}

fn split_host_and_port(host_port: &str) -> (&str, Option<&str>) {
    // We intentionally do not treat bracketed IPv6 as URL-like in this first pass.
    if host_port.starts_with('[') {
        return (host_port, None);
    }

    if let Some((host, port)) = host_port.rsplit_once(':') {
        if !host.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return (host, Some(port));
        }
    }

    (host_port, None)
}

fn is_valid_port(port: &str) -> bool {
    if port.is_empty() || port.len() > 5 || !port.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    port.parse::<u16>().is_ok()
}

fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return false;
    }

    parts
        .iter()
        .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

fn is_domain_name(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if !host.contains('.') {
        return false;
    }

    let mut labels = host.split('.');
    let Some(tld) = labels.next_back() else {
        return false;
    };
    if !is_tld(tld) {
        return false;
    }

    labels.all(is_domain_label)
}

fn is_tld(label: &str) -> bool {
    (2..=63).contains(&label.len()) && label.chars().all(|c| c.is_ascii_alphabetic())
}

fn is_domain_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }

    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(last) = label.chars().next_back() else {
        return false;
    };

    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[derive(Clone, Debug)]
struct MixedUrlWord {
    range: Range<usize>,
    is_url: bool,
}

impl MixedUrlWord {
    fn width(&self, text: &str) -> usize {
        display_width(&text[self.range.clone()])
    }
}

/// The rows of a line mixing URLs and prose, as byte ranges of `text`.
pub(super) fn mixed_url_wrap_ranges(
    text: &str,
    initial_width: usize,
    subsequent_width: usize,
) -> Vec<Range<usize>> {
    let leading_space_width = text.chars().take_while(|ch| *ch == ' ').count();
    let mut words = Vec::new();
    let mut cursor = 0usize;
    for word in WordSeparator::AsciiSpace.find_words(text) {
        let word_start = cursor;
        let word_end = word_start + word.word.len();
        let trailing_space_end = word_end + word.whitespace.len();
        if !word.word.is_empty() {
            words.push(MixedUrlWord {
                range: word_start..word_end,
                is_url: is_url_like_token(word.word),
            });
        }
        cursor = trailing_space_end;
    }

    let mut lines = Vec::new();
    let mut line_start = None;
    let mut line_end = 0usize;
    let mut line_width = 0usize;
    let mut line_limit = initial_width.max(1);

    for word in words {
        let mut pending = split_mixed_url_word(text, word, line_limit);
        let mut pending_idx = 0usize;

        while let Some(piece) = pending.get(pending_idx).cloned() {
            let empty_line_prefix_width = if line_start.is_none() && lines.is_empty() {
                leading_space_width
            } else {
                0
            };
            let empty_line_piece_limit = line_limit.saturating_sub(empty_line_prefix_width).max(1);
            let mut indivisible = false;
            if line_start.is_none() && !piece.is_url && piece.width(text) > empty_line_piece_limit {
                let split = split_mixed_url_word(text, piece.clone(), empty_line_piece_limit);
                if split.len() > 1 {
                    pending.splice(pending_idx..=pending_idx, split);
                    continue;
                }
                indivisible = true;
            }

            let piece_width = piece.width(text);
            let inter_word_space = line_start
                .map(|_| text[line_end..piece.range.start].len())
                .unwrap_or(0);
            let fits = if line_start.is_none() {
                piece.is_url
                    || indivisible
                    || empty_line_prefix_width + piece_width <= line_limit
                    || empty_line_prefix_width >= line_limit
            } else {
                line_width + inter_word_space + piece_width <= line_limit
            };

            if fits {
                if line_start.is_none() {
                    let is_first_output_line = lines.is_empty();
                    let start = if is_first_output_line {
                        0
                    } else {
                        piece.range.start
                    };
                    line_start = Some(start);
                    line_width = if is_first_output_line {
                        leading_space_width + piece_width
                    } else {
                        piece_width
                    };
                } else {
                    line_width += inter_word_space + piece_width;
                }
                line_end = piece.range.end;
                pending_idx += 1;
                continue;
            }

            if let Some(start) = line_start.take() {
                lines.push(start..line_end);
            }
            line_end = 0;
            line_width = 0;
            line_limit = subsequent_width.max(1);
        }
    }

    if let Some(start) = line_start {
        lines.push(start..line_end);
    }

    lines
}

fn split_mixed_url_word(text: &str, word: MixedUrlWord, line_limit: usize) -> Vec<MixedUrlWord> {
    if word.is_url || word.width(text) <= line_limit {
        return vec![word];
    }

    let mut pieces = Vec::new();
    let mut start = word.range.start;
    let mut width = 0usize;
    for (offset, grapheme) in text[word.range.clone()].grapheme_indices(true) {
        let grapheme_width = display_width(grapheme);
        if width > 0 && width + grapheme_width > line_limit.max(1) {
            let end = word.range.start + offset;
            pieces.push(MixedUrlWord {
                range: start..end,
                is_url: false,
            });
            start = end;
            width = 0;
        }
        width += grapheme_width;
    }
    if start < word.range.end {
        pieces.push(MixedUrlWord {
            range: start..word.range.end,
            is_url: false,
        });
    }
    pieces
}
