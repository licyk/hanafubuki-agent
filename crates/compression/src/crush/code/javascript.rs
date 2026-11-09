// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `trimJsTs` and its scanners of `kernel/src/crush.ts` of acp-kernel 0.0.189
// at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use super::Emitter;
use crate::crush::log::split_lines;

enum LineKind {
    Code,
    FullComment,
    BlockOpenOnly,
    BlockOpenMid,
}

/// A `/` can start a regex literal only where an expression is expected:
/// after a token's end it is division.
fn regex_allowed_after(previous: Option<u8>) -> bool {
    match previous {
        None => true,
        Some(byte) => !(byte.is_ascii_alphanumeric() || b"_.'\"`) ]".contains(&byte)),
    }
}

fn previous_significant(line: &[u8], index: usize) -> Option<u8> {
    line[..index]
        .iter()
        .rev()
        .copied()
        .find(|byte| *byte != b' ' && *byte != b'\t')
}

/// The end of a regex literal starting at `index` (past its flags), or
/// `None` when it does not close on the line: then the `/` was division.
fn scan_regex_literal(line: &[u8], index: usize) -> Option<usize> {
    let mut at = index + 1;
    let mut in_class = false;
    while at < line.len() {
        let byte = line[at];
        if byte == b'\\' {
            at += 2;
            continue;
        }
        if in_class {
            in_class = byte != b']';
        } else if byte == b'[' {
            in_class = true;
        } else if byte == b'/' {
            let mut end = at + 1;
            while end < line.len() && line[end].is_ascii_alphabetic() {
                end += 1;
            }
            return Some(end);
        }
        at += 1;
    }
    None
}

fn is_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(u8::is_ascii_whitespace) || String::from_utf8_lossy(bytes).trim().is_empty()
}

// The kernel's scanner never leaves a string: a closing quote is read again
// and reopens it, so nothing after a line's first quote is seen; and its
// template scanner closes at the opening backtick, so a template's text is
// read as code. Both are kept: they only keep more lines.
fn scan_code_line(text: &str) -> LineKind {
    let line = text.as_bytes();
    let mut index = 0;
    while index < line.len() {
        match line[index] {
            b'\'' | b'"' => return LineKind::Code,
            b'/' => match line.get(index + 1) {
                Some(b'/') => {
                    return if is_blank(&line[..index]) {
                        LineKind::FullComment
                    } else {
                        LineKind::Code
                    };
                }
                Some(b'*') => {
                    let before = !is_blank(&line[..index]);
                    let Some(close) = text[index + 2..].find("*/") else {
                        return if before {
                            LineKind::BlockOpenMid
                        } else {
                            LineKind::BlockOpenOnly
                        };
                    };
                    let close = index + 2 + close;
                    if is_blank(&line[close + 2..]) {
                        return if before {
                            LineKind::Code
                        } else {
                            LineKind::FullComment
                        };
                    }
                    index = close + 2;
                    continue;
                }
                // A pattern cannot start with `*`, so only `/*` is surely a
                // comment; any other `/` where an expression may stand is
                // consumed as a whole regex literal, so `/[/*]/` opens no
                // comment.
                _ => {
                    if regex_allowed_after(previous_significant(line, index)) {
                        if let Some(end) = scan_regex_literal(line, index) {
                            index = end;
                            continue;
                        }
                    }
                }
            },
            _ => {}
        }
        index += 1;
    }
    LineKind::Code
}

pub(super) fn trim(text: &str) -> Option<String> {
    let mut out = Emitter::new("//");
    let mut in_block = false;
    let mut blank_run = 0;
    for line in split_lines(text) {
        if in_block {
            match line.find("*/") {
                None => out.elide(),
                Some(close) => {
                    in_block = false;
                    if line[close + 2..].trim().is_empty() {
                        out.elide();
                    } else {
                        out.flush();
                        out.keep(line);
                    }
                    blank_run = 0;
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run >= 3 {
                out.flush();
                out.keep("");
                blank_run = 1;
            }
            continue;
        }
        match scan_code_line(line) {
            LineKind::FullComment => {
                out.elide();
                continue;
            }
            LineKind::BlockOpenOnly => {
                out.elide();
                in_block = true;
                continue;
            }
            LineKind::BlockOpenMid => in_block = true,
            LineKind::Code => {}
        }
        out.flush();
        out.keep(line);
        blank_run = 0;
    }
    if in_block {
        return None;
    }
    out.finish(text)
}
