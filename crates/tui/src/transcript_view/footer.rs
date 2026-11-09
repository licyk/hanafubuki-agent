// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the view puts in its caller's footer: the search query with its
//! caret, and the hints of select mode, a selection, search, a focused
//! activity, and the loading of older history. Each hint has shorter
//! forms for narrow terminals.

use ratatui::style::Stylize;
use ratatui::text::Span;
use ratatui::widgets::StatefulWidget;
use unicode_width::UnicodeWidthStr;

use super::search::Progress;
use super::*;
use crate::bottom_pane::TextAreaState;

/// The footer's rows, and the caret's column on the first one while the
/// query is edited.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TranscriptFooter {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) cursor_column: Option<u16>,
}

impl TranscriptView {
    pub(crate) fn footer(&self, width: u16) -> Option<TranscriptFooter> {
        let has_selected_text = self.has_selection_range();
        if let Some((query, cursor)) = self.search_footer(width) {
            let hint = if has_selected_text {
                selection_hint(width)
            } else {
                self.search_status_line(width)
            };
            return Some(TranscriptFooter {
                lines: vec![query, hint],
                cursor_column: Some(cursor),
            });
        }
        if self.select_mode.is_some() {
            let hints: &[&str] = if has_selected_text {
                &[
                    "arrows/hjkl/w/b/e move · 0/$/g/G ends · pgup/pgdn page · v/V mark · o swap · / find · y copy · enter copy & latest · esc clear · q close",
                    "arrows/hjkl move · o swap · y copy · enter copy & latest · esc clear · q close",
                    "y copy · enter copy & latest · esc clear · q close",
                    "enter copy · q close",
                ]
            } else {
                &[
                    "arrows/hjkl/w/b/e move · 0/$/g/G ends · pgup/pgdn page · v/V/shift+arrows mark · / find · n/N next · enter/q/esc close",
                    "arrows/hjkl/w/b/e move · 0/$/g/G ends · v/V mark · / find · enter/q/esc close",
                    "arrows/hjkl move · v/V mark · / find · enter/q/esc close",
                    "v mark · enter/q close",
                ]
            };
            return Some(single_line(first_fitting_line(
                hints.iter().map(|hint| navigation_line(hint)),
                width,
            )));
        }
        if has_selected_text {
            return Some(single_line(selection_hint(width)));
        }
        if self.search.is_reading() {
            return Some(single_line(self.search_status_line(width)));
        }
        if self.is_activity_focused() {
            return Some(single_line(first_fitting_line(
                [
                    "↑/↓ previous/next · enter details · esc back",
                    "enter details · esc back",
                    "esc back",
                ]
                .map(navigation_line),
                width,
            )));
        }
        let loading = matches!(
            self.history,
            TranscriptHistoryState::LoadingOlder | TranscriptHistoryState::LoadingBeginning
        );
        let failed = self.history == TranscriptHistoryState::Failed;
        let can_return = self.can_return_to_latest() && !self.tail_visible;
        if !(loading || failed || self.unseen_activity || can_return) {
            return None;
        }
        let navigation = if self.can_return_to_latest() {
            "esc latest"
        } else {
            ""
        };
        let status = if loading {
            "↑ Loading earlier messages…"
        } else if failed {
            "Retry history: ctrl+home"
        } else if self.unseen_activity {
            "New activity"
        } else {
            ""
        };
        let joined = match (status.is_empty(), navigation.is_empty()) {
            (false, false) => format!("{status} · {navigation}"),
            (false, true) => status.to_owned(),
            (true, _) => navigation.to_owned(),
        };
        Some(single_line(first_fitting_line(
            [joined.as_str(), navigation, status].map(navigation_line),
            width,
        )))
    }

    /// The query as the editor shows it in one row, and the caret's column.
    fn search_footer(&self, width: u16) -> Option<(Line<'static>, u16)> {
        if !self.is_search_editing() || width == 0 {
            return None;
        }
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        let prefix_width = width.saturating_sub(1).min(6);
        Line::from("Find: ")
            .dim()
            .render(Rect::new(0, 0, prefix_width, 1), &mut buffer);
        let query_area = Rect::new(prefix_width, 0, width - prefix_width, 1);
        let mut state = TextAreaState::default();
        StatefulWidget::render(&self.search.editor, query_area, &mut buffer, &mut state);
        let (cursor_column, _) = self
            .search
            .editor
            .cursor_pos_with_state(query_area, state)?;
        let mut spans = Vec::new();
        let mut column = 0;
        while column < width {
            let cell = &buffer[(column, 0)];
            spans.push(Span::styled(cell.symbol().to_string(), cell.style()));
            column += cell.symbol().width().max(1) as u16;
        }
        Some((Line::from(spans), cursor_column))
    }

    fn search_status_line(&self, width: u16) -> Line<'static> {
        let search = &self.search;
        let reading = search.is_reading();
        let next_hint = "enter accept · ctrl+p older · ctrl+n newer";
        let exhausted_hint = format!("No more matches · {next_hint}");
        let (status, compact) = match search.progress {
            Progress::Idle | Progress::Found if reading => {
                ("ctrl+p older · ctrl+n newer", "ctrl+p older")
            }
            Progress::Idle if !search.editor.is_empty() => (next_hint, "enter accept"),
            Progress::Idle => ("Type to find", "Type to find"),
            Progress::Restart | Progress::Scanning(_) => ("Searching…", "Searching…"),
            Progress::AwaitingHistory if self.history == TranscriptHistoryState::Failed => {
                ("History unavailable · ctrl+p retry", "ctrl+p retry")
            }
            Progress::AwaitingHistory => ("Searching earlier history…", "Loading…"),
            Progress::Found => (next_hint, "enter accept"),
            Progress::Exhausted if reading && search.has_match() => (
                "No more matches · ctrl+p older · ctrl+n newer",
                "ctrl+p older",
            ),
            Progress::Exhausted if search.has_match() => (exhausted_hint.as_str(), "enter accept"),
            Progress::Exhausted => ("No matches", "No matches"),
        };
        let limit = if search.query_truncated {
            " · query limited to 4 KiB"
        } else {
            ""
        };
        let hints = if reading {
            vec![
                format!("Find · {status} · esc latest"),
                format!("Find · {compact} · esc latest"),
                "esc latest".to_owned(),
            ]
        } else {
            vec![
                format!("{status} · full transcript · esc cancel{limit}"),
                format!("{status} · esc cancel{limit}"),
                format!("{compact} · esc cancel"),
                format!("{compact} · esc"),
                "esc cancel".to_owned(),
            ]
        };
        first_fitting_line(hints.iter().map(|hint| navigation_line(hint)), width)
    }
}

fn single_line(line: Line<'static>) -> TranscriptFooter {
    TranscriptFooter {
        lines: vec![line],
        cursor_column: None,
    }
}

fn selection_hint(width: u16) -> Line<'static> {
    first_fitting_line(
        [
            "ctrl+c copy · enter copy & follow · esc clear",
            "enter copy & follow · esc clear",
            "enter copy+↓ · esc",
            "esc clear",
        ]
        .map(navigation_line),
        width,
    )
}

/// The first line that fits, else the last one.
fn first_fitting_line(lines: impl IntoIterator<Item = Line<'static>>, width: u16) -> Line<'static> {
    let mut last = Line::default();
    for line in lines {
        if line.width() <= usize::from(width) {
            return line;
        }
        last = line;
    }
    last
}

/// A hint with its keys plain and its prose dim. Only for hints the
/// interface writes, never for transcript or query text.
fn navigation_line(navigation: &str) -> Line<'static> {
    const ACTIONS: [&str; 22] = [
        " copy & follow",
        " copy & latest",
        " copy+↓",
        " previous/next",
        " move",
        " ends",
        " page",
        " mark",
        " find",
        " swap",
        " accept",
        " cancel",
        " older",
        " newer",
        " latest",
        " retry",
        " details",
        " back",
        " copy",
        " clear",
        " next",
        " close",
    ];
    let mut spans = Vec::new();
    for (index, hint) in navigation.split(" · ").enumerate() {
        if index > 0 {
            spans.push(" · ".dim());
        }
        match ACTIONS.into_iter().find(|action| hint.ends_with(*action)) {
            Some(action) => {
                spans.push(Span::from(hint[..hint.len() - action.len()].to_owned()));
                spans.push(action.dim());
            }
            None if hint == "esc" => spans.push(Span::from("esc")),
            None => spans.push(hint.to_owned().dim()),
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests;
