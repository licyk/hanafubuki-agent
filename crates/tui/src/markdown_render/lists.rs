// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Lists: markers, nesting, spacing and task boxes.
//!
//! - An unordered item's marker is `•` (`-` with `tui.rendering.lists`
//!   off); an ordered one keeps its number.
//! - After an item that took more than one row (wrapped, several
//!   paragraphs, a nested list), its next sibling comes after a blank line.
//! - A task's `☐`/`☑` replaces the bullet, or follows the number; the
//!   hanging indent follows the task's text.

use std::ops::Range;

use pulldown_cmark::{Event, Tag, TagEnd};
use ratatui::text::{Line, Span};

use super::output::spans_display_width;
use super::{preferences, IndentContext, Writer};

impl<'a> Writer<'a> {
    pub(super) fn start_list(&mut self, index: Option<u64>) {
        if self.list_indices.is_empty() && self.needs_newline {
            self.push_line(Line::default());
        }
        self.list_indices.push(index);
        self.list_needs_blank_before_next_item.push(false);
    }

    pub(super) fn end_list(&mut self) {
        self.flush_current_line();
        self.list_indices.pop();
        self.list_needs_blank_before_next_item.pop();
        self.needs_newline = true;
    }

    /// Starts an item, then takes its task marker from the source when the
    /// parser gave none: before block content (`- [x] # title`) or in an
    /// empty item (`- [ ]`).
    pub(super) fn start_item_and_recover_task<I>(&mut self, range: Range<usize>, iter: &mut I)
    where
        I: Iterator<Item = (Event<'a>, Range<usize>)>,
    {
        self.start_item();
        let Some((next, next_range)) = iter.next() else {
            return;
        };
        let content_start = if matches!(next, Event::End(TagEnd::Item)) {
            range.end
        } else {
            next_range.start
        };
        let checked = preferences::current()
            .lists
            .then(|| self.input.get(range.start..content_start))
            .flatten()
            .and_then(|prefix| prefix.lines().next())
            .and_then(|prefix| prefix.trim().split_once(char::is_whitespace))
            .and_then(|(_, marker)| match marker.trim() {
                "[ ]" => Some(false),
                "[x]" | "[X]" => Some(true),
                _ => None,
            });
        if let Some(checked) = checked {
            self.last_task_marker_end = content_start;
            self.task_list_marker(checked);
            if matches!(
                next,
                Event::Start(Tag::List(_) | Tag::HtmlBlock)
                    | Event::End(TagEnd::Item)
                    | Event::Rule
            ) {
                self.push_line(Line::default());
            }
        }
        self.handle_event(next, next_range, iter);
    }

    fn start_item(&mut self) {
        let after_multiline = self
            .list_needs_blank_before_next_item
            .last_mut()
            .map(std::mem::take)
            .unwrap_or(false);
        self.flush_current_line();
        if after_multiline {
            self.push_blank_line();
        }
        self.list_item_start_line_counts.push(self.text.lines.len());
        self.pending_marker_line = true;
        let depth = self.list_indices.len();
        let is_ordered = self
            .list_indices
            .last()
            .map(Option::is_some)
            .unwrap_or(false);
        let width = depth * 4 - 3;
        let bullet = if preferences::current().lists {
            "• "
        } else {
            "- "
        };
        let marker = self
            .list_indices
            .last_mut()
            .map(|last_index| match last_index {
                None => vec![Span::styled(
                    " ".repeat(width - 1) + bullet,
                    self.styles.unordered_list_marker,
                )],
                Some(index) => {
                    *index += 1;
                    vec![Span::styled(
                        format!("{:width$}. ", *index - 1),
                        self.styles.ordered_list_marker,
                    )]
                }
            });
        let indent_prefix = if depth == 0 {
            Vec::new()
        } else {
            let indent_len = if is_ordered { width + 2 } else { width + 1 };
            vec![Span::from(" ".repeat(indent_len))]
        };
        self.indent_stack
            .push(IndentContext::new(indent_prefix, marker, true));
        self.needs_newline = false;
    }

    pub(super) fn end_item(&mut self) {
        if self.pending_marker_line {
            self.push_line(Line::default());
        }
        self.flush_current_line();
        let start_line_count = self.list_item_start_line_counts.pop().unwrap_or_default();
        if self.text.lines.len().saturating_sub(start_line_count) > 1 {
            if let Some(needs_blank) = self.list_needs_blank_before_next_item.last_mut() {
                *needs_blank = true;
            }
        }
        self.indent_stack.pop();
        self.pending_marker_line = false;
    }

    /// The innermost item's marker becomes a task box; the hanging indent
    /// widens to the text after it.
    pub(super) fn task_list_marker(&mut self, checked: bool) {
        let Some(context) = self
            .indent_stack
            .iter_mut()
            .rev()
            .find(|context| context.is_list)
        else {
            return;
        };
        if let Some(marker) = context.marker.as_mut() {
            if let Some(span) = marker.last_mut() {
                let prefix = span.content.strip_suffix("• ").unwrap_or(&span.content);
                let checkbox = if checked { "☑" } else { "☐" };
                span.content = format!("{prefix}{checkbox} ").into();
            }
            context.prefix = vec![Span::from(" ".repeat(spans_display_width(marker)))];
        }
        if self.current_line_content.is_some() {
            // A loose item's paragraph starts before its task marker comes.
            self.current_initial_indent = self.prefix_spans(true);
            self.current_subsequent_indent = self.prefix_spans(false);
        }
    }
}
