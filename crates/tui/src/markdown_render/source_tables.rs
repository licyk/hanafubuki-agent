// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! With `tui.rendering.tables` off, a table, or a ```` ```md ```` fence
//! holding one, is given to the writer as a literal block of its source,
//! before the writer reads its inline Markdown.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::table_detect::is_markdown_fence_info;

type OffsetEvent<'a> = (Event<'a>, Range<usize>);

pub(super) fn preserve<'a>(
    input: &'a str,
    events: impl Iterator<Item = OffsetEvent<'a>>,
) -> impl Iterator<Item = OffsetEvent<'a>> {
    let enabled = super::preferences::current().tables;
    let mut skipped_end = None;
    // Each event gives up to three: none while a literal block's events are
    // skipped, the event itself, or a literal block in place of its start.
    events
        .flat_map(move |(event, range)| -> [Option<OffsetEvent<'a>>; 3] {
            if let Some(end) = skipped_end {
                if matches!(&event, Event::End(tag) if *tag == end) {
                    skipped_end = None;
                }
                return [None, None, None];
            }
            let end = match &event {
                Event::Start(Tag::Table(_)) if !enabled => Some(TagEnd::Table),
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                    if !enabled && is_markdown_fence_info(info, 0) =>
                {
                    Some(TagEnd::CodeBlock)
                }
                _ => None,
            };
            let Some(end) = end else {
                return [Some((event, range)), None, None];
            };
            let source = literal_source(input, range.clone());
            if end == TagEnd::CodeBlock
                && !source.split_once('\n').is_some_and(|(_, body)| {
                    Parser::new_ext(body, Options::ENABLE_TABLES)
                        .any(|event| matches!(event, Event::Start(Tag::Table(_))))
                })
            {
                return [Some((event, range)), None, None];
            }
            skipped_end = Some(end);
            let kind = match event {
                Event::Start(Tag::CodeBlock(kind)) => kind,
                _ => CodeBlockKind::Fenced("".into()),
            };
            [
                Some((Event::Start(Tag::CodeBlock(kind)), range.clone())),
                Some((Event::Text(source.into()), range.clone())),
                Some((Event::End(TagEnd::CodeBlock), range)),
            ]
        })
        .flatten()
}

/// The block's source without the quote and list prefixes of its lines
/// after the first: the writer puts those back once.
fn literal_source(input: &str, range: Range<usize>) -> String {
    // The first line's prefix is outside the parser's range.
    let prefix = input[..range.start].rsplit('\n').next().unwrap_or_default();
    let indent = " ".repeat(prefix.len());
    input[range]
        .split_inclusive('\n')
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                line
            } else if let Some(line) = line.strip_prefix(prefix) {
                line
            } else if let Some((_, suffix)) = prefix.rsplit_once('>') {
                // Only the enclosing quotes are structure; further markers
                // belong to the literal body.
                let mut rest = line;
                for _ in prefix.matches('>') {
                    let Some(content) = rest.trim_start_matches([' ', '\t']).strip_prefix('>')
                    else {
                        break;
                    };
                    rest = content.strip_prefix(' ').unwrap_or(content);
                }
                // Only the list's or the fence's indentation after the last
                // quote marker.
                for _ in 0..suffix.strip_prefix(' ').unwrap_or(suffix).len() {
                    rest = rest.strip_prefix(' ').unwrap_or(rest);
                }
                rest
            } else {
                line.strip_prefix(&indent).unwrap_or(line)
            }
        })
        .collect()
}
