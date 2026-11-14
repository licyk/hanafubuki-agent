// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Math recognised before Markdown reads the source, so its TeX escapes are
//! not Markdown's; the offsets stay the source's.
//!
//! Only the copy the parser reads is masked: each formula becomes a run of
//! `$` (plain text to the parser, never a tag), and its rendering, or its
//! source when it cannot be rendered, takes the run's place among the
//! events as [`Event::InlineMath`], which the writer shows as it is, never
//! scanned for web addresses. Code, links and HTML are left to Markdown. A
//! display that stands on its own lines is followed however far its closer
//! is; one after text on its line only within a bounded lookahead.

use std::borrow::Cow;
use std::iter::Peekable;
use std::ops::Range;
use std::slice;

use pulldown_cmark::{Event, Options, Parser, Tag};

use render::MathMode;

mod render;

const MAX_MATH_BYTES: usize = 4096;

/// The source as the parser reads it, and what replaces each formula.
pub(super) struct MathMarkdown<'a> {
    pub(super) markdown: Cow<'a, str>,
    /// The start of the line of a display still open at the end, which may
    /// close later.
    pub(super) pending_start: Option<usize>,
    /// Each display, from its opener to its closer (or the end), rendered
    /// or not: a block inside one is not complete.
    pub(super) display_ranges: Vec<Range<usize>>,
    replacements: Vec<(Range<usize>, String)>,
}

/// One delimiter pair.
struct Delimiters {
    open: &'static str,
    close: &'static str,
    display: bool,
}

impl<'a> MathMarkdown<'a> {
    pub(super) fn new(input: &'a str, options: Options, width: Option<usize>) -> Self {
        let mut result = Self {
            markdown: Cow::Borrowed(input),
            pending_start: None,
            display_ranges: Vec::new(),
            replacements: Vec::new(),
        };
        if !input.contains('$') && !input.contains("\\(") && !input.contains("\\[") {
            return result;
        }
        let (protected, containers) = protected_ranges(input, options);
        let mut protected = protected.iter().peekable();
        let mut containers = containers.into_iter().peekable();
        let mut offset = 0;
        let mut scanned = 0;
        let mut line_start = 0;
        let mut line_has_text = false;
        while offset < input.len() {
            if let Some(index) = input[scanned..offset].rfind('\n') {
                line_start = scanned + index + 1;
                line_has_text = false;
            }
            line_has_text |= !input[scanned.max(line_start)..offset].trim().is_empty();
            scanned = offset;
            while protected.next_if(|range| range.end <= offset).is_some() {}
            while containers.next_if(|range| range.end <= offset).is_some() {}
            if let Some(range) = protected.peek().filter(|range| range.contains(&offset)) {
                offset = range.end;
                continue;
            }
            let rest = &input[offset..];
            let Some(Delimiters {
                open,
                close,
                display,
            }) = opener(rest)
            else {
                let Some(ch) = rest.chars().next() else {
                    break;
                };
                offset += ch.len_utf8();
                continue;
            };
            let start = offset;
            offset += open.len();
            if escaped(input, start) {
                continue;
            }
            let body = &input[offset..];
            if open == "$"
                && (body.starts_with(char::is_whitespace) || body.starts_with(['(', '{']))
            {
                continue;
            }
            let limit = body
                .char_indices()
                .map(|(index, _)| index)
                .find(|index| *index >= MAX_MATH_BYTES)
                .unwrap_or(body.len());
            let rejected_display = display && line_has_text;
            // Only a display on its own lines may close arbitrarily far on.
            let search = if display && !rejected_display {
                body
            } else {
                &body[..limit]
            };
            let (end, rejected_close) = find_closer(
                input,
                offset,
                search,
                close,
                display.then_some(rejected_display),
                protected.clone(),
            );
            if display && (!rejected_display || end.is_some() || body.len() < MAX_MATH_BYTES) {
                result
                    .display_ranges
                    .push(start..end.map_or(input.len(), |end| end + close.len()));
            }
            // A rejected opener pairs only within the lookahead, so a shell's
            // PID dollars cannot keep a whole streamed answer mutable.
            if rejected_display || rejected_close {
                match end {
                    Some(end) if !input[offset..end].trim().is_empty() => {
                        // A later line's opening `$$` stays its own equation's.
                        if !(rejected_display
                            && open == "$$"
                            && opens_its_own_line(input, offset, end, close))
                        {
                            offset = end + close.len();
                        }
                    }
                    None if open == "\\[" && body.len() < MAX_MATH_BYTES => break,
                    _ => {}
                }
                continue;
            }
            let Some(end) = end else {
                if display {
                    if body.len() < MAX_MATH_BYTES {
                        result.pending_start.get_or_insert(line_start);
                    }
                    result.replace(start..input.len(), input[start..].to_owned());
                    break;
                }
                continue;
            };
            let span = start..end + close.len();
            let formula = &input[offset..end];
            // A display owns its closer, rejected or not.
            if display {
                offset = span.end;
            }
            if protected.peek().is_some_and(|range| range.start < span.end) {
                continue;
            }
            if !display && formula.contains('\n') {
                continue;
            }
            if open == "$" {
                match dollar_verdict(input, formula, span.end) {
                    Dollar::Math => {}
                    Dollar::Retry => continue,
                    Dollar::Skip => {
                        offset = span.end;
                        continue;
                    }
                }
            }
            let mode = if !display {
                MathMode::Inline
            } else if containers
                .peek()
                .is_some_and(|range| range.contains(&start))
            {
                MathMode::NestedDisplay
            } else {
                MathMode::Display
            };
            let rendered = rendered_formula(formula, mode, width)
                .unwrap_or_else(|| input[span.clone()].to_owned());
            offset = span.end;
            result.replace(span, rendered);
        }
        result
    }

    /// `span` masked with `$` in the parser's copy, `text` to take its
    /// place.
    fn replace(&mut self, span: Range<usize>, text: String) {
        self.markdown
            .to_mut()
            .replace_range(span.clone(), &"$".repeat(span.len()));
        self.replacements.push((span, text));
    }

    /// The parser's events with each formula's run of `$` replaced: the
    /// text around it keeps its own event.
    pub(super) fn events<'s>(
        &'s self,
        events: impl Iterator<Item = (Event<'s>, Range<usize>)>,
    ) -> impl Iterator<Item = (Event<'s>, Range<usize>)> {
        let mut replacements = self.replacements.iter().peekable();
        events.flat_map(move |(event, range)| {
            while replacements
                .next_if(|(span, _)| span.end <= range.start)
                .is_some()
            {}
            let masked = matches!(event, Event::Text(_))
                && replacements
                    .peek()
                    .is_some_and(|(span, _)| span.start < range.end);
            if !masked {
                return Pieces::One(Some((event, range)));
            }
            let mut output = Vec::new();
            let mut offset = range.start;
            while let Some((span, text)) = replacements.next_if(|(span, _)| span.end <= range.end) {
                if offset < span.start {
                    output.push((
                        Event::Text(self.markdown[offset..span.start].into()),
                        offset..span.start,
                    ));
                }
                output.push((Event::InlineMath(text.as_str().into()), span.clone()));
                offset = span.end;
            }
            if offset < range.end {
                output.push((
                    Event::Text(self.markdown[offset..range.end].into()),
                    offset..range.end,
                ));
            }
            Pieces::Many(output.into_iter())
        })
    }
}

/// An event as it is, or the pieces of a masked text.
enum Pieces<T> {
    One(Option<T>),
    Many(std::vec::IntoIter<T>),
}

impl<T> Iterator for Pieces<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match self {
            Self::One(item) => item.take(),
            Self::Many(items) => items.next(),
        }
    }
}

/// The delimiters `rest` starts with, the longest first.
fn opener(rest: &str) -> Option<Delimiters> {
    let (open, close, display) = if rest.starts_with("$$") {
        ("$$", "$$", true)
    } else if rest.starts_with("\\[") {
        ("\\[", "\\]", true)
    } else if rest.starts_with("\\(") {
        ("\\(", "\\)", false)
    } else if rest.starts_with('$') {
        ("$", "$", false)
    } else {
        return None;
    };
    Some(Delimiters {
        open,
        close,
        display,
    })
}

/// What Markdown keeps for itself (code, raw HTML, links, images, link
/// definitions), and the lists and quotes, in which a display is nested,
/// each by its start.
fn protected_ranges(input: &str, options: Options) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let parser = Parser::new_ext(input, options);
    let mut protected: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| definition.span.clone())
        .collect();
    let mut containers = Vec::new();
    protected.extend(parser.into_offset_iter().filter_map(|(event, range)| {
        if matches!(event, Event::Start(Tag::List(_) | Tag::BlockQuote(_))) {
            containers.push(range.clone());
        }
        matches!(
            event,
            Event::Code(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::Start(Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. })
        )
        .then_some(range)
    }));
    protected.sort_unstable_by_key(|range| range.start);
    containers.sort_unstable_by_key(|range| range.start);
    (protected, containers)
}

/// The first closer in `search` (the source from `offset` on), and whether
/// a display's closer was rejected for the text after it on its line.
/// `display` is `Some(rejected)` for a display opener: its closer may not
/// fall in protected source, and a standalone display whose closer has
/// text after it is not one if it spans lines.
fn find_closer(
    input: &str,
    offset: usize,
    search: &str,
    close: &str,
    display: Option<bool>,
    mut protected: Peekable<slice::Iter<'_, Range<usize>>>,
) -> (Option<usize>, bool) {
    let mut multiline_close = false;
    let mut rejected_close = false;
    let end = search.match_indices(&close[..1]).find_map(|(index, _)| {
        let end = offset + index;
        if !search[index..].starts_with(close) || escaped(input, end) {
            return None;
        }
        let Some(rejected_display) = display else {
            return Some(end);
        };
        while protected.next_if(|range| range.end <= end).is_some() {}
        if protected
            .peek()
            .is_some_and(|range| range.start < end + close.len())
        {
            return None;
        }
        if !rejected_display && line_has_text_after(input, end + close.len()) {
            if multiline_close || input[offset..end].contains('\n') {
                multiline_close = true;
                return None;
            }
            rejected_close = true;
        }
        Some(end)
    });
    (end, rejected_close)
}

/// A rejected `$$` whose closer begins its own line and has text after
/// it: that closer may open the next equation.
fn opens_its_own_line(input: &str, offset: usize, end: usize, close: &str) -> bool {
    input[offset..end]
        .rsplit_once('\n')
        .is_some_and(|(_, prefix)| prefix.trim().is_empty())
        && line_has_text_after(input, end + close.len())
}

fn line_has_text_after(input: &str, offset: usize) -> bool {
    input[offset..]
        .chars()
        .take_while(|ch| *ch != '\n')
        .any(|ch| !ch.is_whitespace())
}

enum Dollar {
    Math,
    /// Not math; the closer may still open a formula.
    Retry,
    /// Not math, closer included.
    Skip,
}

/// Whether `$formula$`, ending at `after`, is math: not when it ends with
/// a space or a letter or digit follows it (`$5 and $10`, `$HOME/$USER`),
/// nor a number without an operator (`$5; $10`) or an upper-case word
/// (`$USD$`).
fn dollar_verdict(input: &str, formula: &str, after: usize) -> Dollar {
    let next = input[after..].chars().next();
    if formula.ends_with(char::is_whitespace) || next.is_some_and(char::is_alphanumeric) {
        return Dollar::Retry;
    }
    if formula != "0"
        && formula.starts_with(|ch: char| ch.is_ascii_digit())
        && !formula.contains(['\\', '^', '_', '=', '+', '-', '*', '/', '<', '>'])
        || formula.len() > 1 && formula.chars().all(|ch| ch.is_ascii_uppercase())
    {
        return Dollar::Skip;
    }
    Dollar::Math
}

/// `formula` rendered, when math is on and its layout fits: a layout of
/// several rows (or a matrix) only as a display of its own, within the
/// width's last four columns.
fn rendered_formula(formula: &str, mode: MathMode, width: Option<usize>) -> Option<String> {
    if !super::preferences::current().math || formula.len() >= MAX_MATH_BYTES {
        return None;
    }
    let math = render::render(formula, mode)?;
    // A nested display's prefixes take their own width; a spatial layout
    // is never wrapped.
    math.required_width
        .is_none_or(|required| {
            mode == MathMode::Display
                && width.is_none_or(|width| required <= width.saturating_sub(4))
        })
        .then_some(math.text)
}

/// An odd run of backslashes before `offset` escapes it.
fn escaped(input: &str, offset: usize) -> bool {
    input[..offset]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        == 1
}

#[cfg(test)]
mod tests;
