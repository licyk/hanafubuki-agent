// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the person sends while the Agent works (Codex's
//! `chatwidget/input_queue.rs` with `input_flow.rs` and
//! `input_restore.rs`): one queue per conversation, which the chat holds
//! with its composer.
//!
//! - **Enter** while a turn runs steers it: the message is sent to the
//!   turn, one at a time so the turn takes them in order, and waits above
//!   the composer until the turn reads it at its next step; only then is it
//!   written into the conversation. Before the turn has started, or while
//!   the last one settles, Enter queues instead; while a review runs, the
//!   message is held for its end.
//! - **Tab** while a turn runs queues the message; with nothing running it
//!   sends it, as Enter does.
//! - **The turn's end** settles the queue once every steer sent has its
//!   answer. Completed or failed: the steers it did not read and the held
//!   messages are sent together as the next turn, else the oldest queued
//!   message is; one turn at a time. Interrupted by Esc with steers
//!   waiting: those steers are sent at once, as one turn, and none goes to
//!   the turn being interrupted, which refuses them. Interrupted
//!   otherwise (Ctrl-C, Esc with no steer waiting): everything waiting goes
//!   back into the composer, before the draft.
//!
//! A steer's message is kept until the turn's `SteeringConsumed` covers its
//! sequence: the Agent drops the steering a cancelled or failed turn had
//! not read, and refuses one that arrives after the turn ended, and the
//! steer's answer can arrive after the turn's end. Messages sent or put
//! back together are one message, a line each, their images renumbered in
//! order (`UserMessage::join`).

use std::collections::VecDeque;

use crate::agent_bridge::SteerOutcome;
use crate::bottom_pane::PendingInput;
use crate::user_message::UserMessage;

#[derive(Default)]
pub(super) struct InputQueue {
    /// A message started a turn that has not started yet (Codex's
    /// `user_turn_pending_start`).
    pending_start: bool,
    /// A review runs: Enter holds the message for its end.
    review: bool,
    /// Sent with Enter during the turn, oldest first, until it reads them.
    steers: Vec<Steer>,
    /// Sent at the end of the turn, together (Codex's rejected steers).
    held: Vec<UserMessage>,
    /// Queued with Tab: each starts a turn of its own.
    queued: VecDeque<UserMessage>,
    /// Esc with steers waiting: once the turn is interrupted, they are
    /// sent at once; until then none is sent to it.
    send_steers_after_interrupt: bool,
    /// The running turn read its steering through this sequence.
    consumed_through: Option<u64>,
    /// The turn (or the review) ended; the queue settles once no steer
    /// waits for its answer.
    ended: Option<Ending>,
    next_id: u64,
}

struct Steer {
    id: u64,
    message: UserMessage,
    state: SteerState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SteerState {
    /// Behind another steer, or not delivered to a turn that ended.
    Waiting,
    /// Sent; its answer has not come.
    Sent,
    /// The turn took it under this sequence.
    Accepted(u64),
}

/// How a turn or a review ended, for the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ending {
    Finished,
    Interrupted,
}

/// What the chat does with a message the person sent.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Submission {
    /// Starts a turn with it.
    Start(UserMessage),
    /// Steers the running turn with it.
    Steer { id: u64, message: UserMessage },
    /// It waits in the queue.
    Waits,
}

/// What the queue asks of the chat once a turn has settled.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Settled {
    /// A turn starts with this message.
    Start(UserMessage),
    /// These messages go back into the composer, before the draft.
    Restore(UserMessage),
}

/// What Esc does while a turn runs.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum EscAction {
    Interrupt,
    /// A review would be cancelled with messages held for it: say so
    /// instead (Codex's warning).
    WarnReview,
}

impl InputQueue {
    /// A message sent with Enter; `running` while a turn (or a review)
    /// runs.
    pub(super) fn submit(&mut self, message: UserMessage, running: bool) -> Submission {
        if self.review {
            self.held.push(message);
            return Submission::Waits;
        }
        if running && self.ended.is_none() {
            let id = self.next_id;
            self.next_id += 1;
            self.steers.push(Steer {
                id,
                message,
                state: SteerState::Waiting,
            });
            return match self.next_steer() {
                Some((id, message)) => Submission::Steer { id, message },
                None => Submission::Waits,
            };
        }
        self.queue(message, running)
    }

    /// A message queued with Tab.
    pub(super) fn queue(&mut self, message: UserMessage, running: bool) -> Submission {
        if running || self.review || self.pending_start || self.ended.is_some() {
            self.queued.push_back(message);
            return Submission::Waits;
        }
        self.pending_start = true;
        Submission::Start(message)
    }

    /// The steer to send next: the oldest waiting, once none is in flight
    /// and the turn still runs.
    pub(super) fn next_steer(&mut self) -> Option<(u64, UserMessage)> {
        if self.ended.is_some()
            || self.send_steers_after_interrupt
            || self
                .steers
                .iter()
                .any(|steer| steer.state == SteerState::Sent)
        {
            return None;
        }
        let steer = self
            .steers
            .iter_mut()
            .find(|steer| steer.state == SteerState::Waiting)?;
        steer.state = SteerState::Sent;
        Some((steer.id, steer.message.clone()))
    }

    /// The answer to steer `id`: its message when the turn has read it
    /// already, to be written into the conversation.
    pub(super) fn steered(&mut self, id: u64, outcome: SteerOutcome) -> Option<UserMessage> {
        let index = self.steers.iter().position(|steer| steer.id == id)?;
        match outcome {
            SteerOutcome::Accepted(sequence)
                if self
                    .consumed_through
                    .is_some_and(|through| sequence <= through) =>
            {
                Some(self.steers.remove(index).message)
            }
            SteerOutcome::Accepted(sequence) => {
                self.steers[index].state = SteerState::Accepted(sequence);
                None
            }
            // Once the turn ended, or Esc is interrupting it (which can
            // refuse the steer before its end arrives), it is one of the
            // steers it did not read.
            SteerOutcome::NotDelivered
                if self.ended.is_some() || self.send_steers_after_interrupt =>
            {
                self.steers[index].state = SteerState::Waiting;
                None
            }
            SteerOutcome::NotDelivered => {
                let steer = self.steers.remove(index);
                self.held.push(steer.message);
                None
            }
        }
    }

    /// The turn read its steering through `through`: the steers it read,
    /// oldest first, to be written into the conversation.
    pub(super) fn consumed(&mut self, through: u64) -> Vec<UserMessage> {
        self.consumed_through = Some(self.consumed_through.map_or(through, |t| t.max(through)));
        let (read, waiting): (Vec<Steer>, Vec<Steer>) = std::mem::take(&mut self.steers)
            .into_iter()
            .partition(|steer| matches!(steer.state, SteerState::Accepted(s) if s <= through));
        self.steers = waiting;
        read.into_iter().map(|steer| steer.message).collect()
    }

    pub(super) fn turn_started(&mut self) {
        self.pending_start = false;
        self.consumed_through = None;
    }

    pub(super) fn turn_ended(&mut self, ending: Ending) {
        self.pending_start = false;
        self.ended = Some(ending);
    }

    pub(super) fn review_started(&mut self) {
        self.review = true;
    }

    pub(super) fn review_ended(&mut self, ending: Ending) {
        self.review = false;
        self.ended = Some(ending);
    }

    /// Esc while a turn runs: with steers waiting, the interrupt sends
    /// them at once.
    pub(super) fn esc(&mut self) -> EscAction {
        if self.review && !self.held.is_empty() {
            return EscAction::WarnReview;
        }
        self.send_steers_after_interrupt = !self.steers.is_empty();
        EscAction::Interrupt
    }

    /// Whether the interrupt that ends the turn sends the steers at once.
    pub(super) fn sends_steers_after_interrupt(&self) -> bool {
        self.send_steers_after_interrupt && !self.steers.is_empty()
    }

    /// Settles the queue after a turn's end, once no steer waits for its
    /// answer.
    pub(super) fn settle(&mut self) -> Option<Settled> {
        if self
            .steers
            .iter()
            .any(|steer| steer.state == SteerState::Sent)
        {
            return None;
        }
        let ending = self.ended.take()?;
        let send_steers = std::mem::take(&mut self.send_steers_after_interrupt);
        let unread: Vec<UserMessage> = self.steers.drain(..).map(|steer| steer.message).collect();
        match ending {
            Ending::Interrupted if send_steers && !unread.is_empty() => {
                self.pending_start = true;
                Some(Settled::Start(UserMessage::join(unread)))
            }
            Ending::Interrupted => {
                let waiting: Vec<UserMessage> = self
                    .held
                    .drain(..)
                    .chain(unread)
                    .chain(self.queued.drain(..))
                    .collect();
                (!waiting.is_empty()).then(|| Settled::Restore(UserMessage::join(waiting)))
            }
            Ending::Finished => {
                self.held.extend(unread);
                let message = if self.held.is_empty() {
                    self.queued.pop_front()?
                } else {
                    UserMessage::join(std::mem::take(&mut self.held))
                };
                self.pending_start = true;
                Some(Settled::Start(message))
            }
        }
    }

    /// Shift+Left: the last queued message (else the last held one), to
    /// edit in the composer.
    pub(super) fn pop_queued(&mut self) -> Option<UserMessage> {
        self.queued.pop_back().or_else(|| self.held.pop())
    }

    /// Whether anything waits, or a turn is about to start: editing an
    /// earlier message waits for it.
    pub(super) fn is_busy(&self) -> bool {
        self.pending_start
            || self.ended.is_some()
            || !(self.steers.is_empty() && self.held.is_empty() && self.queued.is_empty())
    }

    pub(super) fn preview(&self) -> PendingInput {
        let text = |message: &UserMessage| message.text.clone();
        PendingInput {
            steers: self
                .steers
                .iter()
                .map(|steer| text(&steer.message))
                .collect(),
            held: self.held.iter().map(text).collect(),
            queued: self.queued.iter().map(text).collect(),
        }
    }
}

#[cfg(test)]
mod tests;
