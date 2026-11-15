// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The chat's streams of an answer and of a proposed plan (Codex's
//! `chatwidget/streaming.rs`): their deltas, the commit ticks, a width
//! change, and their end. Every change reaches the app as one
//! `StreamUpdate`; a delta that queues rows starts the commit animation and
//! commits at once while the backlog is caught up.

use std::time::Instant;

use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::markdown_render::ListSpacing;
use crate::streaming::commit_tick::{run_commit_tick, CommitTickScope};
use crate::streaming::controller::{
    Finalized, PlanStreamController, StreamController, StreamUpdate,
};

impl ChatWidget {
    pub(super) fn on_message_delta(&mut self, delta: String) {
        self.restore_retry_status_header();
        if self.stream_controller.is_none() {
            self.stream_controller = Some(StreamController::new(
                self.stream_width(2),
                self.directory.clone(),
                self.streaming_list_spacing(),
            ));
        }
        let Some(controller) = self.stream_controller.as_mut() else {
            return;
        };
        controller.push(&delta);
        let queued = controller.queued_lines() > 0;
        self.after_push(queued);
    }

    pub(super) fn on_plan_delta(&mut self, delta: String) {
        self.restore_retry_status_header();
        if self.plan_stream_controller.is_none() {
            self.plan_stream_controller = Some(PlanStreamController::new(
                self.stream_width(4),
                self.directory.clone(),
                self.streaming_list_spacing(),
            ));
        }
        let Some(controller) = self.plan_stream_controller.as_mut() else {
            return;
        };
        controller.push(&delta);
        let queued = controller.queued_lines() > 0;
        self.after_push(queued);
    }

    /// After a delta: when it queued rows, the animation starts and a
    /// catch-up commits at once, in the same update as the tail.
    fn after_push(&mut self, queued: bool) {
        if queued {
            self.app_event_tx.send(AppEvent::StartCommitAnimation);
            self.run_commit_tick(CommitTickScope::CatchUpOnly, Instant::now());
        } else {
            for update in [
                self.stream_controller
                    .as_mut()
                    .and_then(|controller| controller.update(None)),
                self.plan_stream_controller
                    .as_mut()
                    .and_then(|controller| controller.update(None)),
            ] {
                self.send_stream_update(update);
            }
        }
    }

    /// One step of the commit animation.
    pub(crate) fn on_commit_tick(&mut self) {
        self.on_commit_tick_at(Instant::now());
    }

    /// [`Self::on_commit_tick`] as of `now`, which the pacing reads.
    pub(crate) fn on_commit_tick_at(&mut self, now: Instant) {
        self.run_commit_tick(CommitTickScope::AnyMode, now);
    }

    fn run_commit_tick(&mut self, scope: CommitTickScope, now: Instant) {
        let output = run_commit_tick(
            &mut self.adaptive_chunking,
            self.stream_controller.as_mut(),
            self.plan_stream_controller.as_mut(),
            scope,
            now,
        );
        if let Some(controller) = self.stream_controller.as_mut() {
            let update = controller.update(output.answer);
            self.send_stream_update(update);
        }
        if let Some(controller) = self.plan_stream_controller.as_mut() {
            let update = controller.update(output.plan);
            self.send_stream_update(update);
        }
    }

    /// Whether no stream has a row queued, so the commit animation may
    /// stop.
    pub(crate) fn stream_queues_idle(&self) -> bool {
        self.stream_controller
            .as_ref()
            .is_none_or(|controller| controller.queued_lines() == 0)
            && self
                .plan_stream_controller
                .as_ref()
                .is_none_or(|controller| controller.queued_lines() == 0)
    }

    /// The streams follow the width the chat was last drawn at: what they
    /// queued and their tails are laid out again.
    pub(crate) fn sync_stream_width(&mut self) {
        let answer_width = self.stream_width(2);
        let plan_width = self.stream_width(4);
        if let Some(controller) = self
            .stream_controller
            .as_mut()
            .filter(|controller| controller.width() != answer_width)
        {
            controller.set_width(answer_width);
            let update = controller.update(None);
            self.send_stream_update(update);
        }
        if let Some(controller) = self
            .plan_stream_controller
            .as_mut()
            .filter(|controller| controller.width() != plan_width)
        {
            controller.set_width(plan_width);
            let update = controller.update(None);
            self.send_stream_update(update);
        }
    }

    /// Commits what is left of the streaming message at once; its cells
    /// then give way to the one of its Markdown.
    pub(super) fn finish_message(&mut self) {
        if let Some(mut controller) = self.stream_controller.take() {
            self.add_finalized(controller.finalize());
        }
    }

    /// [`Self::finish_message`] for the proposed plan.
    pub(super) fn finish_plan(&mut self) {
        if let Some(mut controller) = self.plan_stream_controller.take() {
            self.add_finalized(controller.finalize());
        }
    }

    fn add_finalized(&mut self, finalized: Finalized) {
        self.send_stream_update(finalized.update);
        if let Some(consolidation) = finalized.consolidation {
            self.app_event_tx
                .send(AppEvent::ConsolidateStream(consolidation));
        }
        if self.stream_queues_idle() {
            self.adaptive_chunking.reset();
        }
    }

    fn send_stream_update(&mut self, update: Option<StreamUpdate>) {
        let Some(update) = update else {
            return;
        };
        if update.committed.is_some() {
            self.close_tool_group();
        }
        self.app_event_tx.send(AppEvent::StreamUpdate(update));
    }

    /// The columns a stream renders in: the chat's last width less the
    /// stream's indent.
    fn stream_width(&self, indent: usize) -> Option<usize> {
        self.last_rendered_width
            .get()
            .map(|width| width.saturating_sub(indent))
    }

    /// Fullscreen lays a finished answer out again with its lists spaced
    /// uniformly, so it streams them compact; the scrollback keeps the
    /// spacing of the rows it was written.
    fn streaming_list_spacing(&self) -> ListSpacing {
        if self.owned_transcript {
            ListSpacing::Compact
        } else {
            ListSpacing::AfterMultiline
        }
    }
}
