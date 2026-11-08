// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The overlay that puts the model's questions to the person.
//!
//! - Each question is answered by selecting one of its options (↑/↓ or its number).
//! - Enter commits the selection and moves to the next question; on the last one it submits
//!   every answer.
//! - ←/→ move between questions, keeping their selections.
//! - Esc or Ctrl-C interrupts the turn: an unanswered question cannot be skipped.
//!
//! The Agent's questions take option answers only (`AgentUserInputAnswer` names a question and
//! an option), so Codex's notes, its free-form answers, its "None of the above" option and its
//! confirmation for unanswered questions are not ported.

use std::collections::VecDeque;

use hanafubuki_agent::{
    AgentUserInputAnswer, AgentUserInputQuestion, AgentUserInputRequest, AgentUserInputRequestId,
};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;

use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::bottom_pane::scroll_state::ScrollState;
use crate::bottom_pane::selection_popup_common::{
    measure_rows_height, menu_surface_inset, menu_surface_padding_height, render_menu_surface,
    render_rows, GenericDisplayRow,
};
use crate::bottom_pane::CancellationEvent;
use crate::render::renderable::Renderable;
use crate::text_formatting::strip_terminal_controls;

const TIP_SEPARATOR: &str = " | ";

pub(crate) struct RequestUserInputOverlay {
    app_event_tx: AppEventSender,
    request: AgentUserInputRequest,
    // Queue of incoming requests to process after the current one.
    queue: VecDeque<AgentUserInputRequest>,
    // One selection state per question.
    answers: Vec<ScrollState>,
    current_idx: usize,
    done: bool,
}

impl RequestUserInputOverlay {
    pub(crate) fn new(request: AgentUserInputRequest, app_event_tx: AppEventSender) -> Self {
        let mut overlay = Self {
            app_event_tx,
            request,
            queue: VecDeque::new(),
            answers: Vec::new(),
            current_idx: 0,
            done: false,
        };
        overlay.reset_for_request();
        overlay
    }

    fn reset_for_request(&mut self) {
        self.answers = self
            .request
            .questions
            .iter()
            .map(|question| {
                let mut state = ScrollState::new();
                if !question.options.is_empty() {
                    state.selected_idx = Some(0);
                }
                state
            })
            .collect();
        self.current_idx = 0;
    }

    fn current_question(&self) -> Option<&AgentUserInputQuestion> {
        self.request.questions.get(self.current_idx)
    }

    fn question_count(&self) -> usize {
        self.request.questions.len()
    }

    fn options_len(&self) -> usize {
        self.current_question()
            .map(|question| question.options.len())
            .unwrap_or(0)
    }

    fn move_selection(&mut self, down: bool) {
        let len = self.options_len();
        if let Some(state) = self.answers.get_mut(self.current_idx) {
            if down {
                state.move_down_wrap(len);
            } else {
                state.move_up_wrap(len);
            }
            state.ensure_visible(len, len.max(1));
        }
    }

    fn move_question(&mut self, next: bool) {
        let count = self.question_count();
        if count == 0 {
            return;
        }
        self.current_idx = if next {
            (self.current_idx + 1) % count
        } else {
            (self.current_idx + count - 1) % count
        };
    }

    fn go_next_or_submit(&mut self) {
        if self.current_idx + 1 >= self.question_count() {
            self.submit_answers();
        } else {
            self.current_idx += 1;
        }
    }

    /// Answers every question with its selected option; a question with no
    /// options has nothing to answer with.
    fn submit_answers(&mut self) {
        let answers = self
            .request
            .questions
            .iter()
            .zip(&self.answers)
            .filter_map(|(question, state)| {
                let option = question.options.get(state.selected_idx?)?;
                Some(AgentUserInputAnswer {
                    question_id: question.id.clone(),
                    option_id: option.id.clone(),
                })
            })
            .collect();
        self.app_event_tx.send(AppEvent::Op(Op::UserInputAnswer {
            request_id: self.request.request_id,
            answers,
        }));
        self.advance_queue();
    }

    fn advance_queue(&mut self) {
        if let Some(next) = self.queue.pop_front() {
            self.request = next;
            self.reset_for_request();
        } else {
            self.done = true;
        }
    }

    fn option_rows(&self) -> Vec<GenericDisplayRow> {
        let Some(question) = self.current_question() else {
            return Vec::new();
        };
        let selected = self
            .answers
            .get(self.current_idx)
            .and_then(|state| state.selected_idx);
        question
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
            .collect()
    }

    fn question_lines(&self) -> Vec<Line<'static>> {
        let Some(question) = self.current_question() else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        let header = strip_terminal_controls(&question.header);
        if !header.is_empty() {
            lines.push(Line::from(header.bold()));
        }
        lines.push(Line::from(
            strip_terminal_controls(&question.question).cyan(),
        ));
        lines
    }

    fn footer_line(&self) -> Line<'static> {
        let count = self.question_count();
        let mut tips = vec![if self.current_idx + 1 >= count {
            "enter to submit".cyan().bold().not_dim()
        } else {
            "enter to answer".into()
        }];
        if count > 1 {
            tips.push(TIP_SEPARATOR.into());
            tips.push("←/→ questions".into());
        }
        tips.push(TIP_SEPARATOR.into());
        tips.push("esc to interrupt".into());
        Line::from(tips).dim()
    }

    fn content_height(&self, width: u16) -> u16 {
        let question_height = Paragraph::new(self.question_lines())
            .wrap(Wrap { trim: false })
            .line_count(width) as u16;
        let rows = self.option_rows();
        let state = self
            .answers
            .get(self.current_idx)
            .copied()
            .unwrap_or_default();
        let rows_height = measure_rows_height(&rows, &state, rows.len().max(1), width);
        // Progress, question, a blank row, the options, a blank row, the tips.
        1 + question_height + 1 + rows_height + 1 + 1
    }
}

impl BottomPaneView for RequestUserInputOverlay {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Release {
            return;
        }
        match key_event.code {
            KeyCode::Esc => {
                self.on_ctrl_c();
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(false),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(true),
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(false)
            }
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(true)
            }
            KeyCode::Left => self.move_question(false),
            KeyCode::Right | KeyCode::Tab => self.move_question(true),
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let len = self.options_len();
                let idx = ch
                    .to_digit(10)
                    .and_then(|digit| (digit as usize).checked_sub(1))
                    .filter(|idx| *idx < len);
                if let (Some(idx), Some(state)) = (idx, self.answers.get_mut(self.current_idx)) {
                    state.selected_idx = Some(idx);
                    self.go_next_or_submit();
                }
            }
            KeyCode::Enter => self.go_next_or_submit(),
            _ => {}
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.app_event_tx.send(AppEvent::Op(Op::Interrupt));
        self.queue.clear();
        self.done = true;
        CancellationEvent::Handled
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn try_consume_user_input_request(
        &mut self,
        request: AgentUserInputRequest,
    ) -> Option<AgentUserInputRequest> {
        self.queue.push_back(request);
        None
    }

    fn dismiss_user_input(&mut self, request_id: AgentUserInputRequestId) {
        self.queue
            .retain(|request| request.request_id != request_id);
        if self.request.request_id == request_id && !self.done {
            self.advance_queue();
        }
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
        let mut lines = Vec::new();
        // Progress header keeps the user oriented across multiple questions.
        let total = self.question_count();
        lines.push(Line::from(
            format!("Question {}/{total}", self.current_idx + 1).dim(),
        ));
        lines.extend(self.question_lines());
        let header = Paragraph::new(lines).wrap(Wrap { trim: false });
        let header_height = (header.line_count(content.width) as u16).min(content.height);
        header.render(
            Rect {
                height: header_height,
                ..content
            },
            buf,
        );

        let rows = self.option_rows();
        let state = self
            .answers
            .get(self.current_idx)
            .copied()
            .unwrap_or_default();
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
            &state,
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

#[cfg(test)]
mod tests;
