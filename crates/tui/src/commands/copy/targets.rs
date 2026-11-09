// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The parts of an answer `/copy` offers besides the whole of it: each
//! fenced code block, as written, and each top-level quote without its
//! `>` markers (Codex's `markdown::extract_copy_targets`). An indented
//! code block is not offered; a quote with no prose (only code) is not
//! either, its code block is.

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CopyTarget {
    Code {
        language: Option<String>,
        content: String,
    },
    Quote(String),
}

/// The code blocks and quotes of `markdown`, in the order they begin.
pub(crate) fn extract_copy_targets(markdown: &str) -> Vec<CopyTarget> {
    let mut targets = Vec::new();
    // The outermost quote being read: its source range, where it goes in
    // `targets`, and whether it was added (once it has prose).
    let mut quote: Option<(std::ops::Range<usize>, usize, bool)> = None;
    let mut quote_depth = 0usize;
    let mut in_code_block = false;
    let mut code: Option<(Option<String>, String)> = None;

    for (event, range) in Parser::new(markdown).into_offset_iter() {
        match event {
            Event::Start(Tag::BlockQuote(_)) => {
                if quote_depth == 0 {
                    quote = Some((range, targets.len(), false));
                }
                quote_depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                quote_depth = quote_depth.saturating_sub(1);
                if quote_depth == 0 {
                    quote = None;
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                if let CodeBlockKind::Fenced(info) = kind {
                    let language = info
                        .split([',', ' ', '\t'])
                        .next()
                        .filter(|language| !language.is_empty())
                        .map(str::to_owned);
                    code = Some((language, String::new()));
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if in_code_block {
                    if let Some((_, content)) = &mut code {
                        // The parser splits a `\r\n` line end; keep it whole.
                        if text.starts_with('\n')
                            && range.start > 0
                            && markdown.as_bytes()[range.start - 1] == b'\r'
                        {
                            content.push('\r');
                        }
                        content.push_str(&text);
                    }
                } else if let Some((span, position, added)) = &mut quote {
                    if !*added && !text.trim().is_empty() {
                        let content = markdown[span.clone()]
                            .split_inclusive('\n')
                            .map(|line| {
                                line.trim_start_matches(' ')
                                    .strip_prefix('>')
                                    .map_or(line, |line| line.strip_prefix(' ').unwrap_or(line))
                            })
                            .collect::<String>();
                        targets.insert(*position, CopyTarget::Quote(content));
                        *added = true;
                    }
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                if let Some((language, content)) = code.take() {
                    targets.push(CopyTarget::Code { language, content });
                }
            }
            _ => {}
        }
    }
    targets
}
