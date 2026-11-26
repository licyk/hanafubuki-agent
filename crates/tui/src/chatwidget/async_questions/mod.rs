// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).
//! A model's optional questions sit beside the input queue. Each owns its answer
//! draft until the person submits, skips, or the turn finishes.

mod editor;

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use hanafubuki_agent::AgentAsyncQuestion;

use super::ChatWidget;
use crate::bottom_pane::TextArea;
use crate::text_formatting::strip_terminal_controls;
use crate::user_message::UserMessage;
use editor::Editor;

const VIEW: &str = "request_user_input_async";

#[derive(Default)]
pub(super) struct Questions(Rc<RefCell<State>>);

#[derive(Default)]
struct State {
    seen: HashSet<String>,
    answered: HashSet<String>,
    pending: VecDeque<Pending>,
    index: usize,
    submit: Option<bool>,
    expanded: bool,
}

struct Pending {
    id: String,
    call_id: String,
    title: String,
    options: Vec<String>,
    selected: usize,
    draft: TextArea,
    expires: Option<Instant>,
}

impl Pending {
    fn reply(&self) -> Option<String> {
        let answer = self
            .options
            .get(self.selected)
            .map(String::as_str)
            .unwrap_or_else(|| self.draft.text())
            .trim();
        if answer.is_empty() {
            return None;
        }
        Some(hanafubuki_agent::AgentAsyncQuestionReply::message(
            &self.id,
            &self.title,
            answer,
        ))
    }
}

impl Questions {
    pub(super) fn expanded(&self) -> bool {
        self.0.borrow().expanded
    }
    pub(super) fn summary(&self) -> usize {
        self.0.borrow().pending.len()
    }
    pub(super) fn deadline(&self) -> Option<Instant> {
        let now = Instant::now();
        self.0
            .borrow()
            .pending
            .iter()
            .filter_map(|q| q.expires)
            .filter(|&d| d > now)
            .min()
    }
}

impl ChatWidget {
    pub(super) fn receive_async_questions(
        &mut self,
        call: String,
        questions: Vec<AgentAsyncQuestion>,
    ) {
        {
            let mut state = self.async_questions.0.borrow_mut();
            if !state.seen.insert(call.clone()) {
                return;
            }
            let expires = (!state.expanded).then(|| Instant::now() + Duration::from_secs(30));
            for (index, question) in questions.into_iter().enumerate() {
                let id = AgentAsyncQuestion::item_id(&call, index);
                if state.answered.contains(&id) || state.answered.contains(&call) {
                    continue;
                }
                state.pending.push_back(Pending {
                    id,
                    call_id: call.clone(),
                    title: strip_terminal_controls(&question.title),
                    options: question
                        .options
                        .unwrap_or_default()
                        .iter()
                        .map(|s| strip_terminal_controls(s))
                        .collect(),
                    selected: 0,
                    draft: TextArea::new(),
                    expires,
                });
            }
        }
        self.sync_pending_input();
    }

    pub(super) fn open_async_question(&mut self) -> bool {
        if self.async_questions.summary() == 0 {
            return false;
        }
        let mut state = self.async_questions.0.borrow_mut();
        state.expanded = true;
        for question in &mut state.pending {
            question.expires = None;
        }
        drop(state);
        self.bottom_pane
            .show_command_view(VIEW, Box::new(Editor::new(self.async_questions.0.clone())));
        true
    }

    pub(super) fn take_async_answer(&mut self) {
        let submission = {
            let mut state = self.async_questions.0.borrow_mut();
            let Some(queue) = state.submit.take() else {
                drop(state);
                self.sync_pending_input();
                return;
            };
            state
                .pending
                .get(state.index)
                .and_then(Pending::reply)
                .map(|reply| (reply, queue))
        };
        if let Some((reply, queue)) = submission {
            if self.send_user_message(UserMessage::from_text(reply).into(), queue) {
                let mut state = self.async_questions.0.borrow_mut();
                state.remove_current();
                let empty = state.pending.is_empty();
                drop(state);
                if empty {
                    self.bottom_pane.close_command_views(VIEW);
                }
                self.sync_pending_input();
            }
        }
    }

    pub(super) fn resolve_async_questions(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let mut state = self.async_questions.0.borrow_mut();
        state.answered.extend(ids.iter().cloned());
        let remains = |q: &Pending| !ids.contains(&q.id) && !ids.contains(&q.call_id);
        let next = state
            .pending
            .iter()
            .take(state.index)
            .filter(|q| remains(q))
            .count();
        state.pending.retain(remains);
        state.index = if next < state.pending.len() { next } else { 0 };
        if state.expanded && state.pending.is_empty() {
            state.expanded = false;
            drop(state);
            self.bottom_pane.close_command_views(VIEW);
        }
    }

    pub(super) fn reset_async_questions(&mut self) {
        if self.async_questions.expanded() {
            self.bottom_pane.close_command_views(VIEW);
        }
        self.async_questions = Questions::default();
    }

    pub(super) fn finish_async_questions(&mut self) {
        let (pending, expanded) = {
            let mut state = self.async_questions.0.borrow_mut();
            state.index = 0;
            state.submit = None;
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.expanded),
            )
        };
        if pending.is_empty() {
            return;
        }
        if expanded {
            self.bottom_pane.close_command_views(VIEW);
        }
        let drafts: Vec<_> = pending
            .iter()
            .filter(|q| !q.draft.text().trim().is_empty())
            .map(|q| {
                format!(
                    "> {}\n\n{}",
                    AgentAsyncQuestion::quoted_title(&q.title),
                    q.draft.text()
                )
            })
            .collect();
        if !drafts.is_empty() {
            self.bottom_pane
                .append_question_drafts(&drafts.join("\n\n"));
        }
        self.sync_pending_input();
    }
}

impl State {
    fn remove_current(&mut self) {
        self.pending.remove(self.index);
        if self.pending.is_empty() {
            self.expanded = false;
        }
        if self.index >= self.pending.len() {
            self.index = 0;
        }
    }
}
