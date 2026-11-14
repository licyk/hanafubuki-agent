// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What a stream needs besides the rows, collected in the writer's one pass
//! over the events: where the last top-level block starts, which a stream
//! renders again as lines arrive while it keeps the blocks before it; and
//! the starts of what may still change the rows already rendered. Offsets
//! index the very `input` rendered: a caller that changed its source first
//! maps them back.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

use super::{preferences, source_tables, ListSpacing, WebLinkDisplay, Writer};
use crate::table_detect::is_markdown_fence_info;
use crate::terminal_hyperlinks::HyperlinkLine;

/// The rows, and where the source may still change them.
pub(crate) struct StreamingMarkdownRender {
    pub(crate) lines: Vec<HyperlinkLine>,
    /// The last top-level block's start, when one came before it.
    pub(crate) last_top_level_block_start: Option<usize>,
    /// A reference definition can change any block, before or after it.
    pub(crate) has_reference_link_definition: bool,
    /// The first block is raw HTML, which follows the blocks before it
    /// without a blank row.
    pub(crate) first_top_level_block_is_html: bool,
    /// The last top-level block holds a ```` ```md ```` fence: what it
    /// shows is decided when it closes (unwrapped around a table, or the
    /// table's source with tables off), so it stays the stream's to change.
    pub(crate) mutable_fence_start: Option<usize>,
    /// The last row ends with a local file link's target, which a `:` on
    /// the next line joins.
    pub(crate) trailing_local_link: bool,
}

/// `input` rendered as [`super::render_markdown_lines`] does,
/// with what a stream needs to know of its blocks.
pub(crate) fn render_streaming_markdown(
    input: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    list_spacing: ListSpacing,
) -> StreamingMarkdownRender {
    render_with_display(
        input,
        width,
        cwd,
        list_spacing,
        preferences::web_link_display(),
    )
}

/// [`render_streaming_markdown`], web links shown as `display` says.
pub(super) fn render_with_display(
    input: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    list_spacing: ListSpacing,
    display: WebLinkDisplay,
) -> StreamingMarkdownRender {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.set(Options::ENABLE_TASKLISTS, preferences::current().lists);
    let parser = Parser::new_ext(input, options);
    let has_reference_link_definition = parser.reference_definitions().iter().next().is_some();
    let mut events = TopLevelBlockTracker {
        iter: source_tables::preserve(input, parser.into_offset_iter()),
        depth: 0,
        block_count: 0,
        last_start: 0,
        first_is_html: false,
        mutable_fence_start: None,
    };
    let mut writer = Writer::new(input, width, cwd, display);
    writer.list_spacing = list_spacing;
    writer.run(&mut events);
    StreamingMarkdownRender {
        lines: writer.lines,
        last_top_level_block_start: (events.block_count > 1).then_some(events.last_start),
        has_reference_link_definition,
        first_top_level_block_is_html: events.first_is_html,
        mutable_fence_start: events.mutable_fence_start,
        trailing_local_link: writer.trailing_local_link,
    }
}

/// Notes the top-level blocks as the writer reads the events.
struct TopLevelBlockTracker<I> {
    iter: I,
    depth: usize,
    block_count: usize,
    last_start: usize,
    first_is_html: bool,
    mutable_fence_start: Option<usize>,
}

impl<'a, I> Iterator for TopLevelBlockTracker<I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    type Item = (Event<'a>, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        let (event, range) = self.iter.next()?;
        if self.depth == 0 && matches!(&event, Event::Start(_) | Event::Rule | Event::Html(_)) {
            self.mutable_fence_start = None;
            self.block_count += 1;
            self.last_start = range.start;
            if self.block_count == 1 {
                self.first_is_html =
                    matches!(&event, Event::Start(Tag::HtmlBlock) | Event::Html(_));
            }
        }
        // Codex holds a Markdown fence only with tables off; an open one
        // with tables on is unwrapped or not only once it closes, too.
        if let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) = &event {
            if is_markdown_fence_info(info, 0) {
                self.mutable_fence_start.get_or_insert(self.last_start);
            }
        }
        match event {
            Event::Start(_) => self.depth += 1,
            Event::End(_) => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        Some((event, range))
    }
}
