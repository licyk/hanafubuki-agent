// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Links. A web link's label takes the link style and carries its
//! destination as a terminal hyperlink; the destination is shown after it
//! in parentheses unless the terminal is one known to open such links
//! ([`WebLinkDisplay`]), or when the label shows nothing. A bare web URL in
//! prose is styled and linked the same way. A local file link shows its
//! target, relative to the session's directory, after its label when the
//! label says something; any other link, its destination after its label.

use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

use super::local_links::{is_local_path_like_link, render_local_link_target};
use super::output::display_width;
use super::Writer;
use crate::terminal_detection::{TerminalInfo, TerminalName};
use crate::terminal_hyperlinks::{annotate_web_urls_in_line, web_destination, HyperlinkLine};

/// Whether a web link shows its label alone, its destination being the
/// hyperlink's, or its destination after it too. A heuristic over the
/// terminal's identity, not a negotiation: an unknown terminal or a
/// multiplexer keeps the destination, so a label never hides the only
/// usable URL.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum WebLinkDisplay {
    LabelOnly,
    #[default]
    WithDestination,
}

impl WebLinkDisplay {
    /// For `terminal`, when the interface's output is a terminal and
    /// GNU screen (`STY`) is not in between.
    pub(crate) fn for_terminal(terminal: &TerminalInfo, is_terminal: bool, screen: bool) -> Self {
        let term = terminal.term.as_deref();
        if !is_terminal
            || screen
            || terminal.multiplexer.is_some()
            || term.is_some_and(|term| {
                term == "dumb" || term.starts_with("screen") || term.starts_with("tmux")
            })
        {
            return Self::WithDestination;
        }
        match terminal.name {
            TerminalName::Ghostty
            | TerminalName::Iterm2
            | TerminalName::WezTerm
            | TerminalName::Rex
            | TerminalName::Kitty
            | TerminalName::VsCode
            | TerminalName::Alacritty
            | TerminalName::WindowsTerminal
            | TerminalName::Konsole
            | TerminalName::GnomeTerminal
            | TerminalName::Vte => Self::LabelOnly,
            TerminalName::AppleTerminal
            | TerminalName::WarpTerminal
            | TerminalName::Dumb
            | TerminalName::Unknown => Self::WithDestination,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct LinkState {
    destination: String,
    /// The sanitised destination of a web link.
    web: Option<String>,
    /// The destination goes after the label.
    show_destination: bool,
    /// The label takes the link style from the start (a label-only link).
    style_label: bool,
    has_visible_label: bool,
    /// A local file link's target as shown; its label's spans are held
    /// until the link closes, to go before the target when they say
    /// something.
    local_target_display: Option<String>,
    local_label_spans: Vec<Span<'static>>,
}

impl Writer<'_> {
    pub(super) fn push_link(&mut self, destination: String) {
        let web = web_destination(&destination);
        let style_label = web.is_some() && self.link_display == WebLinkDisplay::LabelOnly;
        if style_label {
            self.push_inline_style(self.styles.link);
        }
        let local_target_display = is_local_path_like_link(&destination).then(|| {
            render_local_link_target(&destination, self.cwd).unwrap_or_else(|| destination.clone())
        });
        self.link = Some(LinkState {
            show_destination: !style_label && local_target_display.is_none(),
            style_label,
            has_visible_label: false,
            web,
            local_target_display,
            local_label_spans: Vec::new(),
            destination,
        });
    }

    pub(super) fn pop_link(&mut self) {
        let Some(link) = self.link.take() else {
            return;
        };
        if link.style_label {
            self.pop_inline_style();
        }
        if link.show_destination || (!link.has_visible_label && link.web.is_some()) {
            let mut destination = HyperlinkLine::default();
            destination.push_span(
                Span::styled(link.destination, self.styles.link),
                link.web.as_deref(),
            );
            self.push_link_part(HyperlinkLine::from(Line::from(" (")));
            self.push_link_part(destination);
            self.push_link_part(HyperlinkLine::from(Line::from(")")));
        } else if let Some(target) = link.local_target_display {
            self.push_local_link(target, link.local_label_spans);
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
                self.push_link_part(Line::from(span).into());
            }
            self.push_link_part(Line::from(" (").into());
        }
        self.push_link_part(Line::from(target).into());
        if show_label {
            self.push_link_part(Line::from(")").into());
        }
        if !self.in_table_cell() {
            self.line_ends_with_local_link_target = true;
        }
    }

    /// Text written after the link closed, to the cell being read or to
    /// the line.
    fn push_link_part(&mut self, part: HyperlinkLine) {
        if self.in_table_cell() {
            self.push_to_table_cell(part);
            return;
        }
        if self.current_line_content.is_none() {
            self.push_line(Line::default());
        }
        if let Some(line) = self.current_line_content.as_mut() {
            line.append(part);
        }
    }

    /// `span` in the link style when it is a web link's label.
    pub(super) fn style_link_label(&mut self, mut span: Span<'static>) -> Span<'static> {
        if let Some(link) = self.link.as_mut().filter(|link| link.web.is_some()) {
            link.has_visible_label |=
                !span.content.trim().is_empty() && display_width(&span.content) > 0;
            span.style = span.style.patch(self.styles.link);
        }
        span
    }

    /// The destination of the web link being read.
    pub(super) fn web_link_destination(&self) -> Option<String> {
        self.link.as_ref().and_then(|link| link.web.clone())
    }

    /// `span`'s bare web URLs in the link style and linked, the text
    /// around them as it was.
    pub(super) fn style_bare_web_urls(&self, span: Span<'static>) -> HyperlinkLine {
        let mut line = annotate_web_urls_in_line(Line::from(span));
        if line.hyperlinks.is_empty() {
            return line;
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut links = line.hyperlinks.iter().peekable();
        let mut column = 0;
        let span = &line.line.spans[0];
        for grapheme in span.content.graphemes(true) {
            while links.peek().is_some_and(|link| link.columns.end <= column) {
                links.next();
            }
            let style = if links
                .peek()
                .is_some_and(|link| link.columns.contains(&column))
            {
                span.style.patch(self.styles.link)
            } else {
                span.style
            };
            match spans.last_mut() {
                Some(last) if last.style == style => last.content.to_mut().push_str(grapheme),
                _ => spans.push(Span::styled(grapheme.to_owned(), style)),
            }
            column += display_width(grapheme);
        }
        line.line.spans = spans;
        line
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

#[cfg(test)]
mod tests;
