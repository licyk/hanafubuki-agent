// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Markdown as the transcript shows it: pulldown-cmark's events turned into
//! styled lines, wrapped to a width when one is given.
//!
//! - A fenced code block that names its language is highlighted with the
//!   active syntax theme.
//! - Pipe tables are laid out in columns (`tables`), or as key/value records
//!   when the columns no longer read (`tables/key_value.rs`).
//! - Task lists show `☐`/`☑` and unordered items `•` (`lists`); after an
//!   item that took more than one row, its next sibling is a blank line
//!   below.
//! - A web link shows its label in the link style, linked to its
//!   destination, and the destination after it unless the terminal opens
//!   such links; a bare web URL is styled and linked too; a local file link
//!   shows its target, relative to the session's directory (`links`,
//!   `local_links`). Each row carries its links beside its text
//!   ([`HyperlinkLine`]).
//! - `tui.rendering` turns tables or list markers off (`preferences`); what
//!   is off is shown as written.

mod blocks;
mod links;
mod list_spacing;
mod lists;
mod local_links;
mod output;
pub(crate) mod preferences;
mod source_tables;
mod streaming;
mod tables;

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Tag, TagEnd};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

pub(crate) use links::WebLinkDisplay;
pub(crate) use list_spacing::ListSpacing;
pub(crate) use streaming::{render_streaming_markdown, StreamingMarkdownRender};

use crate::terminal_hyperlinks::HyperlinkLine;
use links::LinkState;
use list_spacing::UniformList;
use tables::TableState;

struct MarkdownStyles {
    h1: Style,
    h2: Style,
    h3: Style,
    h4: Style,
    h5: Style,
    h6: Style,
    code: Style,
    emphasis: Style,
    strong: Style,
    strikethrough: Style,
    ordered_list_marker: Style,
    unordered_list_marker: Style,
    link: Style,
    blockquote: Style,
}

impl Default for MarkdownStyles {
    fn default() -> Self {
        Self {
            h1: Style::new().bold().underlined(),
            h2: Style::new().bold(),
            h3: Style::new().bold().italic(),
            h4: Style::new().italic(),
            h5: Style::new().italic(),
            h6: Style::new().italic(),
            code: Style::new().cyan(),
            emphasis: Style::new().italic(),
            strong: Style::new().bold(),
            strikethrough: Style::new().crossed_out(),
            ordered_list_marker: Style::new().light_blue(),
            unordered_list_marker: Style::new(),
            link: Style::new().fg(crate::style::accent_color()).underlined(),
            blockquote: Style::new().green(),
        }
    }
}

#[derive(Clone, Debug)]
struct IndentContext {
    prefix: Vec<Span<'static>>,
    marker: Option<Vec<Span<'static>>>,
    is_list: bool,
}

impl IndentContext {
    fn new(prefix: Vec<Span<'static>>, marker: Option<Vec<Span<'static>>>, is_list: bool) -> Self {
        Self {
            prefix,
            marker,
            is_list,
        }
    }
}

#[cfg(test)]
pub fn render_markdown_text(input: &str) -> ratatui::text::Text<'static> {
    render_markdown_text_with_width(input, None)
}

#[cfg(test)]
pub(crate) fn render_markdown_text_with_width(
    input: &str,
    width: Option<usize>,
) -> ratatui::text::Text<'static> {
    render_markdown_text_with_width_and_cwd(input, width, None)
}

#[cfg(test)]
pub(crate) fn render_markdown_text_with_width_and_cwd(
    input: &str,
    width: Option<usize>,
    cwd: Option<&str>,
) -> ratatui::text::Text<'static> {
    render_markdown_text_with_spacing(input, width, cwd, ListSpacing::AfterMultiline)
}

#[cfg(test)]
pub(crate) fn render_markdown_text_with_spacing(
    input: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    list_spacing: ListSpacing,
) -> ratatui::text::Text<'static> {
    ratatui::text::Text::from(crate::terminal_hyperlinks::visible_lines(
        render_markdown_lines(input, width, cwd, list_spacing),
    ))
}

/// [`render_markdown_lines`]'s visible lines, for a cell that shows no
/// hyperlink: each web link keeps its destination in its text, whatever
/// the terminal.
pub(crate) fn render_markdown_unlinked(
    input: &str,
    width: Option<usize>,
    cwd: Option<&str>,
) -> Vec<Line<'static>> {
    let rendered = streaming::render_with_display(
        input,
        width,
        cwd,
        ListSpacing::AfterMultiline,
        WebLinkDisplay::WithDestination,
    );
    crate::terminal_hyperlinks::visible_lines(rendered.lines)
}

/// `input` rendered, wrapped to `width` when there is one (`None` keeps
/// each line, and each table, at its natural width), its lists spaced by
/// `list_spacing`. `cwd`, the session's directory as it is shown, shortens
/// the targets of local file links under it; without one an absolute
/// target stays absolute.
pub(crate) fn render_markdown_lines(
    input: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    list_spacing: ListSpacing,
) -> Vec<HyperlinkLine> {
    render_streaming_markdown(input, width, cwd, list_spacing).lines
}

/// The pulldown-cmark event consumer: inline styles, the indentation of
/// quotes and lists, list numbering, the link and the table being read.
struct Writer<'a> {
    input: &'a str,
    lines: Vec<HyperlinkLine>,
    styles: MarkdownStyles,
    inline_styles: Vec<Style>,
    indent_stack: Vec<IndentContext>,
    list_indices: Vec<Option<u64>>,
    /// Per open list: whether its last item took more than one row.
    list_needs_blank_before_next_item: Vec<bool>,
    /// Per open item: the rows written before it started.
    list_item_start_line_counts: Vec<usize>,
    list_spacing: ListSpacing,
    /// Per open list, under [`ListSpacing::Uniform`].
    uniform_lists: Vec<UniformList>,
    link: Option<LinkState>,
    link_display: WebLinkDisplay,
    needs_newline: bool,
    pending_marker_line: bool,
    /// Where the last task marker taken ended, so a marker the parser
    /// repeats in a later paragraph of a recovered item is not taken twice.
    last_task_marker_end: usize,
    in_paragraph: bool,
    in_code_block: bool,
    /// The language of the fenced block being read; its text is collected
    /// in `code_block_buffer` and highlighted at its end.
    code_block_lang: Option<String>,
    code_block_buffer: String,
    wrap_width: Option<usize>,
    cwd: Option<&'a str>,
    current_line_content: Option<HyperlinkLine>,
    current_initial_indent: Vec<Span<'static>>,
    current_subsequent_indent: Vec<Span<'static>>,
    current_line_style: Style,
    current_line_in_code_block: bool,
    table_state: Option<TableState>,
    /// The line being built ends with a local file link's target.
    line_ends_with_local_link_target: bool,
    /// A soft break after a local file link, decided by what follows.
    pending_local_link_soft_break: bool,
    /// The last row written ended with a local file link's target: a
    /// stream's next line may still join it.
    trailing_local_link: bool,
}

impl<'a> Writer<'a> {
    fn new(
        input: &'a str,
        wrap_width: Option<usize>,
        cwd: Option<&'a str>,
        link_display: WebLinkDisplay,
    ) -> Self {
        Self {
            input,
            lines: Vec::new(),
            styles: MarkdownStyles::default(),
            inline_styles: Vec::new(),
            indent_stack: Vec::new(),
            list_indices: Vec::new(),
            list_needs_blank_before_next_item: Vec::new(),
            list_item_start_line_counts: Vec::new(),
            list_spacing: ListSpacing::default(),
            uniform_lists: Vec::new(),
            link: None,
            link_display,
            needs_newline: false,
            pending_marker_line: false,
            last_task_marker_end: 0,
            in_paragraph: false,
            in_code_block: false,
            code_block_lang: None,
            code_block_buffer: String::new(),
            wrap_width,
            cwd,
            current_line_content: None,
            current_initial_indent: Vec::new(),
            current_subsequent_indent: Vec::new(),
            current_line_style: Style::default(),
            current_line_in_code_block: false,
            table_state: None,
            line_ends_with_local_link_target: false,
            pending_local_link_soft_break: false,
            trailing_local_link: false,
        }
    }

    fn run<I>(&mut self, iter: &mut I)
    where
        I: Iterator<Item = (Event<'a>, Range<usize>)>,
    {
        while let Some((event, range)) = iter.next() {
            self.handle_event(event, range, iter);
        }
        self.flush_current_line();
    }

    fn handle_event<I>(&mut self, event: Event<'a>, range: Range<usize>, iter: &mut I)
    where
        I: Iterator<Item = (Event<'a>, Range<usize>)>,
    {
        self.prepare_for_event(&event);
        match event {
            Event::Start(tag) => self.start_tag(tag, range, iter),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.text(text),
            Event::Code(code) => self.code(code),
            Event::SoftBreak => self.soft_break(),
            Event::HardBreak => self.hard_break(),
            Event::Rule => {
                self.flush_current_line();
                if !self.lines.is_empty() {
                    self.push_blank_line();
                }
                self.push_line(Line::from("———"));
                self.needs_newline = true;
            }
            Event::Html(html) => self.html(html, false),
            Event::InlineHtml(html) => self.html(html, true),
            Event::TaskListMarker(checked) => {
                // The parser can repeat a recovered empty item's marker in a
                // later paragraph.
                if range.start >= self.last_task_marker_end {
                    self.last_task_marker_end = range.end;
                    self.task_list_marker(checked);
                    if self.current_line_content.is_none() {
                        self.push_line(Line::default());
                    }
                }
            }
            Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn start_tag<I>(&mut self, tag: Tag<'a>, range: Range<usize>, iter: &mut I)
    where
        I: Iterator<Item = (Event<'a>, Range<usize>)>,
    {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { level, .. } => self.start_heading(level),
            Tag::BlockQuote(_) => self.start_blockquote(),
            Tag::CodeBlock(kind) => {
                let indent = match kind {
                    CodeBlockKind::Fenced(_) => None,
                    CodeBlockKind::Indented => Some(Span::from(" ".repeat(4))),
                };
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => Some(lang.to_string()),
                    CodeBlockKind::Indented => None,
                };
                self.start_codeblock(lang, indent)
            }
            Tag::List(start) => {
                // A list that starts on a line of its own after its parent's
                // marker gives that marker a row.
                if self.pending_marker_line
                    && self.input[..range.start]
                        .rsplit('\n')
                        .next()
                        .is_some_and(|line| line.trim_matches(['>', ' ', '\t']).is_empty())
                {
                    self.push_line(Line::default());
                }
                self.start_list(start);
            }
            Tag::Item => self.start_item_and_recover_task(range, iter),
            Tag::Emphasis => self.push_inline_style(self.styles.emphasis),
            Tag::Strong => self.push_inline_style(self.styles.strong),
            Tag::Strikethrough => self.push_inline_style(self.styles.strikethrough),
            Tag::Link { dest_url, .. } => self.push_link(dest_url.to_string()),
            Tag::Table(alignments) => self.start_table(alignments),
            Tag::TableHead => self.start_table_head(),
            Tag::TableRow => self.start_table_row(range),
            Tag::TableCell => self.start_table_cell(range),
            Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::Image { .. }
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::Heading(_) => self.end_heading(),
            TagEnd::BlockQuote(_) => {
                self.flush_current_line();
                self.end_blockquote();
            }
            TagEnd::CodeBlock => self.end_codeblock(),
            TagEnd::List(_) => self.end_list(),
            TagEnd::Item => self.end_item(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_inline_style(),
            TagEnd::Link => self.pop_link(),
            TagEnd::Table => self.end_table(),
            TagEnd::TableHead => self.end_table_head(),
            TagEnd::TableRow => self.end_table_row(),
            TagEnd::TableCell => self.end_table_cell(),
            TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::Image
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript => {}
        }
    }

    fn push_inline_style(&mut self, style: Style) {
        let current = self.inline_styles.last().copied().unwrap_or_default();
        let merged = current.patch(style);
        self.inline_styles.push(merged);
    }

    fn pop_inline_style(&mut self) {
        self.inline_styles.pop();
    }

    fn current_inline_style(&self) -> Style {
        self.inline_styles.last().copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
