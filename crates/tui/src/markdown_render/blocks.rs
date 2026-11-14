// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Paragraphs, headings, quotes, code blocks and the inline events: each
//! goes to the table cell being read when there is one.

use pulldown_cmark::{CowStr, Event, HeadingLevel};
use ratatui::text::{Line, Span};

use super::{IndentContext, ListSpacing, Writer};
use crate::render::highlight::highlight_code_to_lines;

impl Writer<'_> {
    pub(super) fn start_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline {
            self.push_blank_line();
        }
        self.push_line(Line::default());
        self.needs_newline = false;
        self.in_paragraph = true;
    }

    pub(super) fn end_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.needs_newline = true;
        self.in_paragraph = false;
        self.pending_marker_line = false;
    }

    pub(super) fn start_heading(&mut self, level: HeadingLevel) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline {
            self.push_line(Line::default());
            self.needs_newline = false;
        }
        let heading_style = match level {
            HeadingLevel::H1 => self.styles.h1,
            HeadingLevel::H2 => self.styles.h2,
            HeadingLevel::H3 => self.styles.h3,
            HeadingLevel::H4 => self.styles.h4,
            HeadingLevel::H5 => self.styles.h5,
            HeadingLevel::H6 => self.styles.h6,
        };
        let content = format!("{} ", "#".repeat(level as usize));
        self.push_line(Line::from(vec![Span::styled(content, heading_style)]));
        self.push_inline_style(heading_style);
        self.needs_newline = false;
    }

    pub(super) fn end_heading(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.needs_newline = true;
        self.pop_inline_style();
    }

    pub(super) fn start_blockquote(&mut self) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        self.indent_stack
            .push(IndentContext::new(vec![Span::from("> ")], None, false));
    }

    pub(super) fn end_blockquote(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.indent_stack.pop();
        self.needs_newline = true;
    }

    pub(super) fn text(&mut self, text: CowStr<'_>) {
        self.text_run(text, true);
    }

    /// A formula as the math masker gave it (its Unicode layout, or its
    /// source): text, never scanned for web addresses.
    pub(super) fn math(&mut self, math: CowStr<'_>) {
        self.text_run(math, false);
    }

    /// Text, its bare web addresses linked when it is `prose`.
    fn text_run(&mut self, text: CowStr<'_>, prose: bool) {
        if self.collecting_local_link_label() {
            let style = self.current_inline_style();
            for (index, line) in text.lines().enumerate() {
                if index > 0 {
                    self.push_local_link_label_break();
                }
                self.push_local_link_label_span(Span::styled(line.to_string(), style));
            }
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            self.push_text_to_table_cell(&text, prose);
            return;
        }
        if self.pending_marker_line {
            self.push_line(Line::default());
        }
        self.pending_marker_line = false;
        // Pulldown-cmark's text events of a fenced block carry its line
        // breaks: they are joined as they are.
        if self.in_code_block && self.code_block_lang.is_some() {
            self.code_block_buffer.push_str(&text);
            return;
        }
        if self.in_code_block && !self.needs_newline {
            let has_content = self
                .current_line_content
                .as_ref()
                .or(self.lines.last())
                .is_some_and(|line| !line.line.spans.is_empty());
            if has_content {
                self.push_line(Line::default());
            }
        }
        for (i, line) in text.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if i > 0 {
                self.push_line(Line::default());
            }
            let span = Span::styled(line.to_string(), self.current_inline_style());
            if prose {
                self.push_text_span(span);
            } else {
                self.push_span(span);
            }
        }
        self.needs_newline = false;
    }

    pub(super) fn code(&mut self, code: CowStr<'_>) {
        let span = Span::from(code.into_string()).style(self.styles.code);
        if self.collecting_local_link_label() {
            self.push_local_link_label_span(span);
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            self.push_span_to_table_cell(span);
            return;
        }
        if self.pending_marker_line {
            self.push_line(Line::default());
            self.pending_marker_line = false;
        }
        self.push_span(span);
    }

    pub(super) fn html(&mut self, html: CowStr<'_>, inline: bool) {
        let style = self.current_inline_style();
        if self.collecting_local_link_label() {
            for (index, line) in html.lines().enumerate() {
                if index > 0 {
                    self.push_local_link_label_break();
                }
                self.push_local_link_label_span(Span::styled(line.to_string(), style));
            }
            if !inline {
                self.push_local_link_label_break();
            }
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            for (index, line) in html.lines().enumerate() {
                if index > 0 {
                    self.push_table_cell_hard_break();
                }
                self.push_span_to_table_cell(Span::styled(line.to_string(), style));
            }
            if !inline {
                self.push_table_cell_hard_break();
            }
            return;
        }
        self.pending_marker_line = false;
        for (i, line) in html.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if i > 0 {
                self.push_line(Line::default());
            }
            self.push_span(Span::styled(line.to_string(), style));
        }
        self.needs_newline = !inline;
    }

    pub(super) fn hard_break(&mut self) {
        if self.collecting_local_link_label() {
            self.push_local_link_label_break();
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            self.push_table_cell_hard_break();
            return;
        }
        self.push_line(Line::default());
    }

    pub(super) fn soft_break(&mut self) {
        if self.collecting_local_link_label() {
            self.push_local_link_label_break();
            return;
        }
        if self.in_table_cell() {
            let span = Span::styled(" ".to_string(), self.current_inline_style());
            self.push_span_to_table_cell(span);
            return;
        }
        // A local file link shows its target where the link ends, so a
        // soft break before a `: description` keeps it on the link's row.
        if self.line_ends_with_local_link_target {
            self.pending_local_link_soft_break = true;
            self.line_ends_with_local_link_target = false;
            return;
        }
        self.push_line(Line::default());
    }

    /// The soft break held after a local link: none before `:`, a new
    /// row before anything else.
    pub(super) fn prepare_for_event(&mut self, event: &Event<'_>) {
        if !self.pending_local_link_soft_break {
            return;
        }
        self.pending_local_link_soft_break = false;
        if !matches!(event, Event::Text(text) if text.trim_start().starts_with(':')) {
            self.push_line(Line::default());
        }
    }

    pub(super) fn start_codeblock(&mut self, lang: Option<String>, indent: Option<Span<'static>>) {
        self.flush_current_line();
        // Under uniform spacing, a code block that starts an item follows
        // its marker without a blank line.
        let first_item_block = self.pending_marker_line
            && self
                .indent_stack
                .last()
                .is_some_and(|context| context.is_list);
        if !self.lines.is_empty()
            && (self.list_spacing == ListSpacing::AfterMultiline || !first_item_block)
        {
            self.push_blank_line();
        }
        self.in_code_block = true;
        // The info string's first word: `rust,no_run` and `rust title=x`
        // are Rust.
        self.code_block_lang = lang
            .as_deref()
            .and_then(|info| info.split([',', ' ', '\t']).next())
            .filter(|lang| !lang.is_empty())
            .map(str::to_string);
        self.code_block_buffer.clear();
        self.indent_stack.push(IndentContext::new(
            vec![indent.unwrap_or_default()],
            None,
            false,
        ));
        self.needs_newline = true;
    }

    pub(super) fn end_codeblock(&mut self) {
        if let Some(lang) = self.code_block_lang.take() {
            let code = std::mem::take(&mut self.code_block_buffer);
            if !code.is_empty() {
                for line in highlight_code_to_lines(&code, &lang) {
                    self.push_line(Line::default());
                    for span in line.spans {
                        self.push_span(span);
                    }
                }
            }
        }
        self.needs_newline = true;
        self.in_code_block = false;
        self.indent_stack.pop();
    }
}
