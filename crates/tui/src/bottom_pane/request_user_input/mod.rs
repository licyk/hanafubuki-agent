// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The overlay that puts the model's questions to the person.
//!
//! - A question is answered by choosing one of its options (↑/↓, its number,
//!   or a click), or, when the question takes one, by writing the answer.
//! - Writing is an explicit move: Enter on **Write my own answer**, or `i`.
//!   Moving the cursor onto that row does nothing, so the options stay
//!   reachable. While writing, every printable key is text — digits, `k`,
//!   `j` and `1`-`9` included — and only Enter (done), Esc (done) and
//!   Backspace mean anything else.
//! - Enter commits an answer and moves to the next question. After the last
//!   one a review page lists every answer with **Submit** and **Cancel**;
//!   Cancel discards them all and interrupts the turn, answering nothing.
//! - ←/→ move between questions, Alt+`1`-`9` jump to one.
//! - A request the model marked as not blocking resolves itself once nobody
//!   has looked at it for a while, so a turn never waits on it forever.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use hanafubuki_agent::{
    AgentUserInputAnswer, AgentUserInputQuestion, AgentUserInputRequest, AgentUserInputRequestId,
    AgentUserInputResponse,
};
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::crossterm::event::KeyModifiers;

use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::bottom_pane::scroll_state::ScrollState;
use crate::bottom_pane::CancellationEvent;
use crate::text_formatting::strip_terminal_controls;

mod render;

const TIP_SEPARATOR: &str = " | ";
/// A row added to a question that takes an answer in the person's own words.
const OTHER_ROW_LABEL: &str = "Write my own answer";
/// How long a request that does not block goes unlooked-at before it starts
/// counting down, and how long the countdown is shown.
const AUTO_RESOLUTION_HIDDEN_GRACE: Duration = Duration::from_secs(60);
const AUTO_RESOLUTION_VISIBLE_COUNTDOWN: Duration = Duration::from_secs(60);

/// Where the overlay is: putting the questions, or the page that submits
/// their answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Questions,
    Review,
}

/// What the review page's cursor is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewChoice {
    Submit,
    Cancel,
}

pub(crate) struct RequestUserInputOverlay {
    app_event_tx: AppEventSender,
    request: AgentUserInputRequest,
    // Queue of incoming requests to process after the current one.
    queue: VecDeque<AgentUserInputRequest>,
    // One answer state per question.
    answers: Vec<AnswerState>,
    stage: Stage,
    review: ScrollState,
    /// When the current request was shown, and whether the person touched it.
    shown_at: Instant,
    touched: bool,
    current_idx: usize,
    done: bool,
}

/// What one question holds while the person works on it: the option under
/// the cursor, the answer in their own words, and whether they are writing
/// it right now.
#[derive(Default)]
struct AnswerState {
    options: ScrollState,
    draft: String,
    writing: bool,
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
        let mut review = ScrollState::new();
        review.selected_idx = Some(0);
        let mut overlay = Self {
            app_event_tx,
            request,
            queue: VecDeque::new(),
            answers: Vec::new(),
            stage: Stage::Questions,
            review,
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
                    draft: String::new(),
                    writing: false,
                }
            })
            .collect();
        self.stage = Stage::Questions;
        self.review.selected_idx = Some(0);
        self.current_idx = 0;
    }

    fn current_question(&self) -> Option<&AgentUserInputQuestion> {
        self.request.questions.get(self.current_idx)
    }

    fn question_count(&self) -> usize {
        self.request.questions.len()
    }

    /// The rows of the current question: its options, then, when it takes
    /// an answer of the person's own, that row.
    fn options_len(&self) -> usize {
        let Some(question) = self.current_question() else {
            return 0;
        };
        question.options.len() + usize::from(question.is_other)
    }

    /// The row of the answer in the person's own words, when the current
    /// question takes one.
    fn own_answer_row(&self) -> Option<usize> {
        self.current_question()
            .is_some_and(|question| question.is_other)
            .then(|| self.options_len().saturating_sub(1))
    }

    /// Whether the person is writing the current question's answer right now.
    fn is_writing(&self) -> bool {
        self.answers
            .get(self.current_idx)
            .is_some_and(|state| state.writing)
    }

    /// Starts writing the current answer, with the cursor on its row.
    fn begin_writing(&mut self) {
        let Some(row) = self.own_answer_row() else {
            return;
        };
        if let Some(state) = self.answers.get_mut(self.current_idx) {
            state.options.selected_idx = Some(row);
            state.writing = true;
        }
        self.touched = true;
    }

    /// Stops writing, keeping what was written.
    fn finish_writing(&mut self) {
        if let Some(state) = self.answers.get_mut(self.current_idx) {
            state.writing = false;
            // An empty answer leaves nothing to submit, so the cursor goes
            // back to the options.
            if state.draft.trim().is_empty() {
                state.options.selected_idx = Some(0);
            }
        }
        self.touched = true;
    }

    fn move_selection(&mut self, down: bool) {
        let len = self.options_len();
        if let Some(state) = self.answers.get_mut(self.current_idx) {
            if down {
                state.options.move_down_wrap(len);
            } else {
                state.options.move_up_wrap(len);
            }
            state.options.ensure_visible(len, len.max(1));
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

    /// Turns a question's state into what the model is told. Words only win
    /// when the question takes them and there are some; otherwise the
    /// chosen option is the answer.
    fn answer_of(
        question: &AgentUserInputQuestion,
        state: &AnswerState,
    ) -> Option<AgentUserInputResponse> {
        if question.is_other && !state.draft.trim().is_empty() {
            return Some(AgentUserInputResponse::Text {
                text: state.draft.trim().to_string(),
            });
        }
        question
            .options
            .get(state.options.selected_idx?)
            .map(|option| AgentUserInputResponse::Option {
                option_id: option.id.clone(),
            })
    }

    /// What a question was answered with, in words, for the review page.
    fn answer_label(&self, index: usize) -> String {
        let Some(question) = self.request.questions.get(index) else {
            return String::new();
        };
        let Some(state) = self.answers.get(index) else {
            return "unanswered".to_string();
        };
        match Self::answer_of(question, state) {
            None => "unanswered".to_string(),
            Some(AgentUserInputResponse::Text { text }) => text,
            Some(AgentUserInputResponse::Option { option_id }) => question
                .options
                .iter()
                .find(|option| option.id == option_id)
                .map(|option| strip_terminal_controls(&option.label))
                .unwrap_or(option_id),
        }
    }

    /// Moves on after an answer: the next question, or the review page after
    /// the last one.
    fn go_next(&mut self) {
        if let Some(state) = self.answers.get_mut(self.current_idx) {
            state.writing = false;
        }
        if self.current_idx + 1 >= self.question_count() {
            self.stage = Stage::Review;
            self.review.selected_idx = Some(0);
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

    /// Chooses the review page's entry: submitting the answers, or
    /// cancelling them.
    fn choose_review(&mut self) {
        match self.review.selected_idx {
            Some(1) => self.cancel_answers(),
            _ => self.submit_answers(),
        }
    }

    /// Answers nothing: the selections are dropped and the turn is
    /// interrupted, since it cannot go on without an answer.
    fn cancel_answers(&mut self) {
        self.answers.clear();
        self.on_ctrl_c();
    }

    fn handle_question_key(&mut self, key_event: KeyEvent) {
        if self.is_writing() {
            self.handle_writing_key(key_event);
            return;
        }
        match key_event.code {
            KeyCode::Esc => {
                self.on_ctrl_c();
            }
            KeyCode::Up => self.move_selection(false),
            KeyCode::Down => self.move_selection(true),
            KeyCode::Char('k') if !key_event.modifiers.contains(KeyModifiers::ALT) => {
                self.move_selection(false)
            }
            KeyCode::Char('j') if !key_event.modifiers.contains(KeyModifiers::ALT) => {
                self.move_selection(true)
            }
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(false)
            }
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(true)
            }
            KeyCode::Left => self.move_question(false),
            KeyCode::Right | KeyCode::Tab => self.move_question(true),
            // `i` asks for the answer in the person's own words: the explicit
            // way in, beside Enter on its row.
            KeyCode::Char('i') if self.own_answer_row().is_some() => self.begin_writing(),
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
                let picked = ch
                    .to_digit(10)
                    .and_then(|digit| (digit as usize).checked_sub(1))
                    .filter(|index| *index < self.options_len());
                let Some(index) = picked else {
                    return;
                };
                if Some(index) == self.own_answer_row() {
                    self.begin_writing();
                    return;
                }
                if let Some(state) = self.answers.get_mut(self.current_idx) {
                    state.options.selected_idx = Some(index);
                }
                self.touched = true;
                self.go_next();
            }
            KeyCode::Enter => {
                let selected = self
                    .answers
                    .get(self.current_idx)
                    .and_then(|state| state.options.selected_idx);
                if selected.is_some() && selected == self.own_answer_row() {
                    self.begin_writing();
                } else {
                    self.touched = true;
                    self.go_next();
                }
            }
            _ => {}
        }
    }

    /// While writing, every printable key is text. The interface's own
    /// bindings — digits, `k`/`j`, `i` — reach nothing else, so an answer
    /// that starts with `1` stays an answer.
    fn handle_writing_key(&mut self, key_event: KeyEvent) {
        let control = key_event
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key_event.code {
            KeyCode::Enter => self.go_next(),
            KeyCode::Esc => self.finish_writing(),
            KeyCode::Tab => self.finish_writing(),
            KeyCode::Backspace => {
                if let Some(state) = self.answers.get_mut(self.current_idx) {
                    state.draft.pop();
                }
                self.touched = true;
            }
            KeyCode::Char(ch) if !control => {
                if let Some(state) = self.answers.get_mut(self.current_idx) {
                    state.draft.push(ch);
                }
                self.touched = true;
            }
            _ => {}
        }
    }

    fn handle_review_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Up => self.review.move_up_wrap(2),
            KeyCode::Down => self.review.move_down_wrap(2),
            KeyCode::Char('k') => self.review.move_up_wrap(2),
            KeyCode::Char('j') => self.review.move_down_wrap(2),
            KeyCode::Char('1') => {
                self.review.selected_idx = Some(0);
                self.choose_review();
            }
            KeyCode::Char('2') => {
                self.review.selected_idx = Some(1);
                self.choose_review();
            }
            KeyCode::Enter => self.choose_review(),
            // Going back keeps every answer: only Cancel drops them.
            KeyCode::Left | KeyCode::Backspace | KeyCode::Esc => {
                self.stage = Stage::Questions;
                self.current_idx = self.question_count().saturating_sub(1);
                self.touched = true;
            }
            KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.on_ctrl_c();
            }
            _ => {}
        }
    }
}

impl BottomPaneView for RequestUserInputOverlay {
    fn pointer_key(
        &mut self,
        mouse: ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyCode> {
        match self.stage {
            Stage::Questions => {
                let index = self.current_idx;
                let state = self.answers.get_mut(index)?;
                // A click selects the row it lands on; writing is Enter's.
                let key = state.options.pointer(mouse, KeyCode::Enter)?;
                self.touched = true;
                Some(key)
            }
            Stage::Review => {
                let key = self.review.pointer(mouse, KeyCode::Enter)?;
                self.touched = true;
                Some(key)
            }
        }
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Release {
            return;
        }
        match self.stage {
            Stage::Questions => self.handle_question_key(key_event),
            Stage::Review => self.handle_review_key(key_event),
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

#[cfg(test)]
mod tests;
