// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `trimPython` of `kernel/src/crush.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use std::sync::LazyLock;

use regex::Regex;

use super::Emitter;
use crate::crush::log::split_lines;

static DEF_OR_CLASS: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^(async\s+)?(def|class)(?-u:\b)").ok());
static TRIPLE_OPEN: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#"^\s*("""|''')"#).ok());

/// Where a docstring may stand: at the top of the module, or first in a
/// `def` or `class` body.
#[derive(Default)]
struct Slots {
    at_module_start: bool,
    pending_def_class: bool,
    docstring_slot: bool,
}

impl Slots {
    /// Reads the code part of a line for the next docstring slot.
    fn see(&mut self, code: &str) {
        let code = code.trim();
        if DEF_OR_CLASS.as_ref().is_some_and(|re| re.is_match(code)) {
            self.pending_def_class = true;
        }
        if code.ends_with(':') {
            self.docstring_slot = self.pending_def_class;
            self.pending_def_class = false;
        } else if !code.is_empty() && !code.starts_with('@') {
            self.docstring_slot = false;
        }
        self.at_module_start = false;
    }

    fn take_docstring_slot(&mut self) -> bool {
        let slot = self.at_module_start || self.docstring_slot;
        self.at_module_start = false;
        self.docstring_slot = false;
        slot
    }
}

pub(super) fn trim(text: &str) -> Option<String> {
    let mut out = Emitter::new("#");
    let mut triple: Option<(&str, bool)> = None;
    let mut blank_run = 0;
    let mut slots = Slots {
        at_module_start: true,
        ..Slots::default()
    };
    for (index, line) in split_lines(text).into_iter().enumerate() {
        if let Some((quote, is_docstring)) = triple {
            let Some(close) = find_triple_close(line, quote) else {
                keep_or_elide(&mut out, line, is_docstring);
                continue;
            };
            triple = None;
            let rest = line[close + 3..].trim();
            if rest.is_empty() {
                keep_or_elide(&mut out, line, is_docstring);
            } else {
                out.flush();
                out.keep(line);
                slots.see(rest);
            }
            slots.docstring_slot = false;
            slots.pending_def_class = false;
            slots.at_module_start = false;
            blank_run = 0;
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
        if index == 0 && line.starts_with("#!") {
            blank_run = 0;
            out.flush();
            out.keep(line);
            continue;
        }
        if let Some(quote) = TRIPLE_OPEN
            .as_ref()
            .and_then(|re| re.captures(line))
            .and_then(|captures| captures.get(1))
        {
            blank_run = 0;
            out.flush();
            triple = open_triple(&mut out, &mut slots, line, quote.start(), quote.as_str());
            continue;
        }
        // A comment line leaves the blank run counting.
        if is_full_comment(line) {
            out.elide();
            continue;
        }
        blank_run = 0;
        out.flush();
        out.keep(line);
        slots.see(line);
    }
    if triple.is_some() {
        return None;
    }
    out.finish(text)
}

fn keep_or_elide(out: &mut Emitter, line: &str, is_docstring: bool) {
    if is_docstring {
        out.elide();
    } else {
        out.keep(line);
    }
}

/// A line starting with a triple quote; the triple string still open after
/// it, if any.
fn open_triple<'a>(
    out: &mut Emitter,
    slots: &mut Slots,
    line: &str,
    at: usize,
    quote: &'a str,
) -> Option<(&'a str, bool)> {
    if let Some(close) = find_triple_close(&line[at + 3..], quote) {
        let rest = line[at + 3 + close + 3..].trim();
        if !rest.is_empty() {
            out.keep(line);
            slots.see(rest);
        } else if slots.at_module_start || slots.docstring_slot {
            out.elide();
            slots.take_docstring_slot();
            slots.pending_def_class = false;
        } else {
            out.keep(line);
        }
        return None;
    }
    let is_docstring = slots.take_docstring_slot();
    if !is_docstring {
        out.keep(line);
    }
    Some((quote, is_docstring))
}

fn find_triple_close(line: &str, quote: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index..].starts_with(quote.as_bytes()) {
            return Some(index);
        }
        index += 1;
    }
    None
}

/// Whether the line is a comment alone. The kernel's scanner never leaves
/// a string (a closing quote reopens it), so only a `#` before any quote
/// counts, and a triple quote in mid-line is never seen; kept.
fn is_full_comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
}
