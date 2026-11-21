// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! How the overlay is drawn: the question or the review page above, its rows
//! in the middle, and the keys that work below.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;

use crate::bottom_pane::scroll_state::ScrollState;
use crate::bottom_pane::selection_popup_common::{
    measure_rows_height, menu_surface_inset, menu_surface_padding_height, render_menu_surface,
    render_rows, GenericDisplayRow,
};
use crate::render::renderable::Renderable;
use crate::text_formatting::strip_terminal_controls;

use super::{
    AutoResolution, RequestUserInputOverlay, ReviewChoice, Stage, OTHER_ROW_LABEL, TIP_SEPARATOR,
};

impl RequestUserInputOverlay {
    fn question_rows(&self) -> Vec<GenericDisplayRow> {
        let Some(question) = self.current_question() else {
            return Vec::new();
        };
        let state = self.answers.get(self.current_idx);
        let selected = state.and_then(|state| state.options.selected_idx);
        let mut rows = question
            .options
            .iter()
            .enumerate()
            .map(|(idx, option)| {
                let prefix = if selected == Some(idx) { '›' } else { ' ' };
                let description = strip_terminal_controls(&option.description);
                GenericDisplayRow {
                    name: format!(
                        "{prefix} {}. {}",
                        idx + 1,
                        strip_terminal_controls(&option.label)
                    ),
                    description: (!description.is_empty()).then_some(description),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        if let Some(row) = self.own_answer_row() {
            let prefix = if selected == Some(row) { '›' } else { ' ' };
            let draft = state.map(|state| strip_terminal_controls(&state.draft));
            let writing = state.is_some_and(|state| state.writing);
            let description = match draft {
                // The caret marks where typing goes.
                Some(draft) if writing => format!("{draft}▏"),
                Some(draft) if !draft.trim().is_empty() => draft,
                _ => "Enter writes one".to_string(),
            };
            rows.push(GenericDisplayRow {
                name: format!("{prefix} {}. {OTHER_ROW_LABEL}", row + 1),
                description: Some(description),
                ..Default::default()
            });
        }
        rows
    }

    fn review_rows(&self) -> Vec<GenericDisplayRow> {
        let selected = self.review.selected_idx;
        [
            (ReviewChoice::Submit, "Submit the answers"),
            (ReviewChoice::Cancel, "Cancel, answering nothing"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (_, label))| GenericDisplayRow {
            name: format!(
                "{} {}. {label}",
                if selected == Some(index) { '›' } else { ' ' },
                index + 1
            ),
            ..Default::default()
        })
        .collect()
    }

    fn header_lines(&self) -> Vec<Line<'static>> {
        match self.stage {
            Stage::Questions => {
                let Some(question) = self.current_question() else {
                    return Vec::new();
                };
                let mut lines = vec![Line::from(
                    format!(
                        "Question {}/{}",
                        self.current_idx + 1,
                        self.question_count()
                    )
                    .dim(),
                )];
                let header = strip_terminal_controls(&question.header);
                if !header.is_empty() {
                    lines.push(Line::from(header.bold()));
                }
                lines.push(Line::from(
                    strip_terminal_controls(&question.question).cyan(),
                ));
                lines
            }
            Stage::Review => {
                let mut lines = vec![Line::from("Review".bold())];
                for (index, question) in self.request.questions.iter().enumerate() {
                    let asked = strip_terminal_controls(&question.question);
                    let answered = self.answer_label(index);
                    lines.push(Line::from(format!("{asked} → {answered}")));
                }
                lines
            }
        }
    }

    fn footer_line(&self) -> Line<'static> {
        let mut tips = Vec::new();
        match self.stage {
            Stage::Questions if self.is_writing() => {
                tips.push("enter to finish writing".cyan().bold().not_dim());
                tips.push(TIP_SEPARATOR.into());
                tips.push("esc to stop writing".into());
            }
            Stage::Questions => {
                tips.push(if self.current_idx + 1 >= self.question_count() {
                    "enter to review".cyan().bold().not_dim()
                } else {
                    "enter to answer".into()
                });
                if self.own_answer_row().is_some() {
                    tips.push(TIP_SEPARATOR.into());
                    tips.push("i to write your own".into());
                }
                if self.question_count() > 1 {
                    tips.push(TIP_SEPARATOR.into());
                    tips.push("←/→ or alt+1-9 questions".into());
                }
                tips.push(TIP_SEPARATOR.into());
                tips.push("esc to interrupt".into());
            }
            Stage::Review => {
                tips.push("enter to choose".cyan().bold().not_dim());
                tips.push(TIP_SEPARATOR.into());
                tips.push("← to go back".into());
                tips.push(TIP_SEPARATOR.into());
                tips.push("ctrl-c to interrupt".into());
            }
        }
        if let AutoResolution::Countdown(left) = self.auto_resolution() {
            tips.push(TIP_SEPARATOR.into());
            tips.push(format!("answers itself in {left}s").yellow());
        }
        Line::from(tips).dim()
    }

    fn rows(&self) -> Vec<GenericDisplayRow> {
        match self.stage {
            Stage::Questions => self.question_rows(),
            Stage::Review => self.review_rows(),
        }
    }

    /// The scroll state the rows are drawn with; `empty` stands in when
    /// there is none.
    fn rows_state<'a>(&'a self, empty: &'a ScrollState) -> &'a ScrollState {
        match self.stage {
            Stage::Questions => self
                .answers
                .get(self.current_idx)
                .map(|state| &state.options)
                .unwrap_or(empty),
            Stage::Review => &self.review,
        }
    }

    fn content_height(&self, width: u16) -> u16 {
        let header_height = Paragraph::new(self.header_lines())
            .wrap(Wrap { trim: false })
            .line_count(width) as u16;
        let rows = self.rows();
        let empty = ScrollState::default();
        let rows_height =
            measure_rows_height(&rows, self.rows_state(&empty), rows.len().max(1), width);
        // The header, a blank row, the rows, a blank row, the tips.
        header_height + 1 + rows_height + 1 + 1
    }
}

impl Renderable for RequestUserInputOverlay {
    fn desired_height(&self, width: u16) -> u16 {
        let inner_width = menu_surface_inset(Rect::new(0, 0, width, u16::MAX)).width;
        self.content_height(inner_width) + menu_surface_padding_height()
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let content = render_menu_surface(area, buf);
        let header = Paragraph::new(self.header_lines()).wrap(Wrap { trim: false });
        let header_height = (header.line_count(content.width) as u16).min(content.height);
        header.render(
            Rect {
                height: header_height,
                ..content
            },
            buf,
        );

        let rows = self.rows();
        let empty = ScrollState::default();
        let state = self.rows_state(&empty);
        let rows_top = content.y + header_height + 1;
        let bottom = content.y + content.height;
        let rows_area = Rect {
            x: content.x,
            y: rows_top.min(bottom),
            width: content.width,
            height: bottom.saturating_sub(rows_top).saturating_sub(2),
        };
        render_rows(
            rows_area,
            buf,
            &rows,
            state,
            rows.len().max(1),
            "No options",
        );
        if content.height >= 1 {
            let footer = Rect {
                y: bottom - 1,
                height: 1,
                ..content
            };
            self.footer_line().render(footer, buf);
        }
    }
}
