// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The overlay that puts the model's questions to the person.
//!
//! - Each question is answered by selecting one of its options (↑/↓ or its number), or, when the
//!   question allows it, by typing an answer of their own (`o`, or its row).
//! - Enter commits the answer and moves to the next question; on the last one it submits every
//!   answer. A question left unanswered is submitted as such.
//! - ←/→ move between questions, and a question's number jumps to it, keeping every answer.
//! - Clicking a row selects it, and clicking the question row focuses it.
//! - Esc or Ctrl-C interrupts the turn.
//!
//! A request the model marked as not blocking is answered with nothing once it has gone
//! unlooked-at for a while, so a turn never waits forever on it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use hanafubuki_agent::{
    AgentUserInputAnswer, AgentUserInputQuestion, AgentUserInputRequest, AgentUserInputRequestId,
    AgentUserInputResponse,
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
/// A row added to a question that takes an answer in the person's own words.
const OTHER_ROW_LABEL: &str = "Write my own answer";
/// How long a request that does not block goes unlooked-at before it starts
/// counting down, and how long the countdown is shown.
const AUTO_RESOLUTION_HIDDEN_GRACE: Duration = Duration::from_secs(60);
const AUTO_RESOLUTION_VISIBLE_COUNTDOWN: Duration = Duration::from_secs(60);

pub(crate) struct RequestUserInputOverlay {
    app_event_tx: AppEventSender,
    request: AgentUserInputRequest,
    // Queue of incoming requests to process after the current one.
    queue: VecDeque<AgentUserInputRequest>,
    // One selection state per question.
    answers: Vec<AnswerState>,
    /// When the current request was shown, and whether the person touched it.
    shown_at: Instant,
    touched: bool,
    current_idx: usize,
    done: bool,
}

/// What one question holds while the person works on it: the option under
/// the cursor, and the answer they are typing.
#[derive(Default)]
struct AnswerState {
    options: ScrollState,
    /// The answer in the person's own words, when they are writing one.
    draft: Option<String>,
}

/// What auto-resolution is doing for the current request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoResolution {
    Disabled,
    HiddenGrace,
    /// The seconds left before the request answers itself.
    Countdown(u64),
}

impl RequestUserInputOverlay {
    pub(crate) fn new(request: AgentUserInputRequest, app_event_tx: AppEventSender) -> Self {
        let mut overlay = Self {
            app_event_tx,
            request,
            queue: VecDeque::new(),
            answers: Vec::new(),
            shown_at: Instant::now(),
            touched: false,
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
                let mut options = ScrollState::new();
                if !question.options.is_empty() {
                    options.selected_idx = Some(0);
                }
                AnswerState {
                    options,
                    draft: None,
                }
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
        let Some(question) = self.current_question() else {
            return 0;
        };
        question.options.len() + usize::from(question.is_other)
    }

    /// Whether the cursor of the current question is on its drafting row.
    fn current_is_drafting(&self) -> bool {
        let Some(question) = self.current_question() else {
            return false;
        };
        let Some(state) = self.answers.get(self.current_idx) else {
            return false;
        };
        question.is_other && state.draft.is_some()
    }

    fn move_selection(&mut self, down: bool) {
        let len = self.options_len();
        let takes_other = self
            .current_question()
            .is_some_and(|question| question.is_other);
        if let Some(state) = self.answers.get_mut(self.current_idx) {
            if down {
                state.options.move_down_wrap(len);
            } else {
                state.options.move_up_wrap(len);
            }
            state.options.ensure_visible(len, len.max(1));
            // Landing on the drafting row starts a draft; leaving it keeps
            // what was typed, so the person can come back to it.
            let drafting = takes_other && state.options.selected_idx == Some(len.saturating_sub(1));
            if drafting && state.draft.is_none() {
                state.draft = Some(String::new());
            }
        }
        // Looking at the question is not ignoring it: once the person has
        // moved around it, it waits for them.
        self.touched = true;
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
        self.touched = true;
    }

    /// Jumps to the question numbered `index` (from zero).
    fn jump_to(&mut self, index: usize) {
        if index < self.question_count() {
            self.current_idx = index;
            self.touched = true;
        }
    }

    /// Turns the current question's answer into what the model is told.
    fn answer_of(
        question: &AgentUserInputQuestion,
        state: &AnswerState,
    ) -> Option<AgentUserInputResponse> {
        if question.is_other {
            if let Some(draft) = state
                .draft
                .as_ref()
                .filter(|draft| !draft.trim().is_empty())
            {
                return Some(AgentUserInputResponse::Text {
                    text: draft.trim().to_string(),
                });
            }
        }
        question
            .options
            .get(state.options.selected_idx?)
            .map(|option| AgentUserInputResponse::Option {
                option_id: option.id.clone(),
            })
    }

    fn go_next_or_submit(&mut self) {
        if self.current_idx + 1 >= self.question_count() {
            self.submit_answers();
        } else {
            self.current_idx += 1;
        }
    }

    /// Answers every question with what the person chose or typed. A
    /// question they left alone is left out, and the model is told it went
    /// unanswered.
    fn submit_answers(&mut self) {
        let answers = self
            .request
            .questions
            .iter()
            .zip(&self.answers)
            .filter_map(|(question, state)| {
                let response = Self::answer_of(question, state)?;
                Some(AgentUserInputAnswer {
                    question_id: question.id.clone(),
                    response,
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
            self.shown_at = Instant::now();
            self.touched = false;
            self.reset_for_request();
        } else {
            self.done = true;
        }
    }

    /// What auto-resolution is doing now. Only a request the model did not
    /// make blocking can answer itself, and only while the person has not
    /// touched it.
    fn auto_resolution(&self) -> AutoResolution {
        if self.request.is_blocking || self.touched {
            return AutoResolution::Disabled;
        }
        let elapsed = self.shown_at.elapsed();
        if elapsed < AUTO_RESOLUTION_HIDDEN_GRACE {
            return AutoResolution::HiddenGrace;
        }
        let counting = elapsed - AUTO_RESOLUTION_HIDDEN_GRACE;
        if counting < AUTO_RESOLUTION_VISIBLE_COUNTDOWN {
            let left = AUTO_RESOLUTION_VISIBLE_COUNTDOWN - counting;
            return AutoResolution::Countdown(left.as_secs() + u64::from(left.subsec_nanos() > 0));
        }
        AutoResolution::Countdown(0)
    }

    /// The frame in which a countdown is due, so the interface wakes for it.
    fn next_frame_delay(&self) -> Option<Duration> {
        match self.auto_resolution() {
            AutoResolution::Disabled => None,
            AutoResolution::HiddenGrace => Some(Duration::from_secs(1)),
            AutoResolution::Countdown(0) => Some(Duration::ZERO),
            AutoResolution::Countdown(left) => Some(Duration::from_secs(left.min(1))),
        }
    }

    /// Answers a request nobody looked at with nothing, and moves on. The
    /// model learns the questions went unanswered.
    fn maybe_auto_resolve(&mut self) -> bool {
        if !matches!(self.auto_resolution(), AutoResolution::Countdown(0)) {
            return false;
        }
        self.app_event_tx.send(AppEvent::Op(Op::UserInputAnswer {
            request_id: self.request.request_id,
            answers: Vec::new(),
        }));
        self.advance_queue();
        true
    }

    fn option_rows(&self) -> Vec<GenericDisplayRow> {
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
        if question.is_other {
            let index = question.options.len();
            let prefix = if selected == Some(index) { '›' } else { ' ' };
            let draft = state
                .and_then(|state| state.draft.as_deref())
                .map(strip_terminal_controls);
            let description = match draft {
                // The caret marks where typing goes.
                Some(draft) if selected == Some(index) => format!("{draft}▏"),
                Some(draft) if !draft.trim().is_empty() => draft,
                _ => "Type your own answer".to_string(),
            };
            rows.push(GenericDisplayRow {
                name: format!("{prefix} {}. {OTHER_ROW_LABEL}", index + 1),
                description: Some(description),
                ..Default::default()
            });
        }
        rows
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
        if self
            .current_question()
            .is_some_and(|question| question.is_other)
            && !self.current_is_drafting()
        {
            tips.push(TIP_SEPARATOR.into());
            tips.push("type to answer freely".into());
        }
        if count > 1 {
            tips.push(TIP_SEPARATOR.into());
            tips.push("←/→ or alt+1-9 questions".into());
        }
        tips.push(TIP_SEPARATOR.into());
        match self.auto_resolution() {
            AutoResolution::Countdown(left) => {
                tips.push(format!("answers itself in {left}s").yellow());
            }
            _ => tips.push("esc to interrupt".into()),
        }
        Line::from(tips).dim()
    }

    fn content_height(&self, width: u16) -> u16 {
        let question_height = Paragraph::new(self.question_lines())
            .wrap(Wrap { trim: false })
            .line_count(width) as u16;
        let rows = self.option_rows();
        let empty = ScrollState::default();
        let state = self
            .answers
            .get(self.current_idx)
            .map(|state| &state.options)
            .unwrap_or(&empty);
        let rows_height = measure_rows_height(&rows, state, rows.len().max(1), width);
        // Progress, question, a blank row, the options, a blank row, the tips.
        1 + question_height + 1 + rows_height + 1 + 1
    }
}

impl BottomPaneView for RequestUserInputOverlay {
    fn pointer_key(
        &mut self,
        mouse: ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyCode> {
        let len = self.options_len();
        let takes_other = self
            .current_question()
            .is_some_and(|question| question.is_other);
        let index = self.current_idx;
        let state = self.answers.get_mut(index)?;
        let key = state.options.pointer(mouse, KeyCode::Enter)?;
        // Clicking the drafting row starts a draft there.
        if takes_other && state.options.selected_idx == Some(len.saturating_sub(1)) {
            state.draft.get_or_insert_with(String::new);
        }
        self.touched = true;
        Some(key)
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Release {
            return;
        }
        let drafting = self.current_is_drafting();
        match key_event.code {
            KeyCode::Esc => {
                self.on_ctrl_c();
            }
            KeyCode::Up if !drafting => self.move_selection(false),
            KeyCode::Down if !drafting => self.move_selection(true),
            KeyCode::Char('k') if !drafting => self.move_selection(false),
            KeyCode::Char('j') if !drafting => self.move_selection(true),
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(false)
            }
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(true)
            }
            KeyCode::Left => self.move_question(false),
            KeyCode::Right | KeyCode::Tab => self.move_question(true),
            // Alt+number jumps to that question; a bare number picks an
            // option, so the two never collide.
            KeyCode::Char(ch)
                if key_event.modifiers.contains(KeyModifiers::ALT) && ch.is_ascii_digit() =>
            {
                if let Some(index) = ch
                    .to_digit(10)
                    .and_then(|digit| (digit as usize).checked_sub(1))
                {
                    self.jump_to(index);
                }
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let len = self.options_len();
                let takes_other = self
                    .current_question()
                    .is_some_and(|question| question.is_other);
                let index = ch
                    .to_digit(10)
                    .and_then(|digit| (digit as usize).checked_sub(1))
                    .filter(|index| *index < len);
                if let (Some(index), Some(state)) = (index, self.answers.get_mut(self.current_idx))
                {
                    state.options.selected_idx = Some(index);
                    if takes_other && index == len.saturating_sub(1) {
                        state.draft.get_or_insert_with(String::new);
                    }
                }
                if index.is_some() {
                    self.touched = true;
                    self.go_next_or_submit();
                }
            }
            KeyCode::Enter => self.go_next_or_submit(),
            KeyCode::Backspace => {
                let Some(state) = self.answers.get_mut(self.current_idx) else {
                    return;
                };
                if let Some(draft) = state.draft.as_mut() {
                    draft.pop();
                    self.touched = true;
                }
            }
            // Any other printable key becomes part of the answer in the
            // person's own words, for a question that takes one.
            KeyCode::Char(ch)
                if self
                    .current_question()
                    .is_some_and(|question| question.is_other)
                    && !key_event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let drafting_now = self.current_is_drafting();
                let drafting_row = self.options_len().saturating_sub(1);
                let Some(state) = self.answers.get_mut(self.current_idx) else {
                    return;
                };
                // Typing anything takes the cursor to the drafting row, so
                // the answer being written is the one that is submitted.
                if !drafting_now {
                    state.options.selected_idx = Some(drafting_row);
                }
                state.draft.get_or_insert_with(String::new).push(ch);
                self.touched = true;
            }
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

    fn next_deadline(&self) -> Option<Instant> {
        self.next_frame_delay().map(|delay| Instant::now() + delay)
    }

    fn on_deadline(&mut self) -> bool {
        self.maybe_auto_resolve()
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
        let empty = ScrollState::default();
        let state = self
            .answers
            .get(self.current_idx)
            .map(|state| &state.options)
            .unwrap_or(&empty);
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

#[cfg(test)]
mod tests;
