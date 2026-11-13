// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Links. A web link's label takes the link style and its destination
//! follows in parentheses, as it does for any other link that is not a
//! local file (no terminal hyperlink carries it yet). A local file link
//! shows its target, relative to the session's directory, after its label
//! when the label says something.

use ratatui::text::{Line, Span};
use url::Url;

use super::local_links::{is_local_path_like_link, render_local_link_target};
use super::Writer;

/// Destinations longer than this are no web destination.
const MAX_WEB_DESTINATION_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug)]
pub(super) struct LinkState {
    destination: String,
    is_web: bool,
    /// A local file link's target as shown; its label's spans are held
    /// until the link closes, to go before the target when they say
    /// something.
    local_target_display: Option<String>,
    local_label_spans: Vec<Span<'static>>,
}

impl Writer<'_> {
    pub(super) fn push_link(&mut self, destination: String) {
        let local_target_display = is_local_path_like_link(&destination).then(|| {
            render_local_link_target(&destination, self.cwd).unwrap_or_else(|| destination.clone())
        });
        self.link = Some(LinkState {
            is_web: is_web_destination(&destination),
            local_target_display,
            local_label_spans: Vec::new(),
            destination,
        });
    }

    pub(super) fn pop_link(&mut self) {
        let Some(link) = self.link.take() else {
            return;
        };
        if let Some(target) = link.local_target_display {
            self.push_local_link(target, link.local_label_spans);
        } else {
            self.push_link_part(" (".into());
            self.push_link_part(Span::styled(link.destination, self.styles.link));
            self.push_link_part(")".into());
        }
    }

    /// A local file link: its label (when it has text) and its target, in
    /// the code style.
    fn push_local_link(&mut self, target: String, label: Vec<Span<'static>>) {
        let show_label = label.iter().any(|span| !span.content.trim().is_empty());
        let target = Span::styled(target, self.current_inline_style().patch(self.styles.code));
        if !self.in_table_cell() && self.pending_marker_line {
            self.push_line(Line::default());
        }
        if show_label {
            for span in label {
                self.push_link_part(span);
            }
            self.push_link_part(" (".into());
        }
        self.push_link_part(target);
        if show_label {
            self.push_link_part(")".into());
        }
        if !self.in_table_cell() {
            self.line_ends_with_local_link_target = true;
        }
    }

    /// A span written after the link closed, to the cell being read or to
    /// the line.
    fn push_link_part(&mut self, span: Span<'static>) {
        if self.in_table_cell() {
            self.push_span_to_table_cell(span);
        } else {
            self.push_span(span);
        }
    }

    /// `span` in the link style when it is a web link's label.
    pub(super) fn style_link_label(&self, mut span: Span<'static>) -> Span<'static> {
        if self.link.as_ref().is_some_and(|link| link.is_web) {
            span.style = span.style.patch(self.styles.link);
        }
        span
    }

    pub(super) fn collecting_local_link_label(&self) -> bool {
        self.link
            .as_ref()
            .is_some_and(|link| link.local_target_display.is_some())
    }

    pub(super) fn push_local_link_label_span(&mut self, span: Span<'static>) {
        if let Some(link) = self.link.as_mut() {
            link.local_label_spans.push(span);
        }
    }

    /// A break in a local link's label is a space.
    pub(super) fn push_local_link_label_break(&mut self) {
        let needs_space = self
            .link
            .as_ref()
            .and_then(|link| link.local_label_spans.last())
            .and_then(|span| span.content.chars().last())
            .is_some_and(|character| !character.is_whitespace());
        if needs_space {
            self.push_local_link_label_span(" ".into());
        }
    }
}

/// Whether `destination`, its control characters removed, is an `http` or
/// `https` URL with a host.
fn is_web_destination(destination: &str) -> bool {
    if destination.len() > MAX_WEB_DESTINATION_BYTES {
        return false;
    }
    let destination: String = destination.chars().filter(|ch| !ch.is_control()).collect();
    Url::parse(&destination).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some()
    })
}
