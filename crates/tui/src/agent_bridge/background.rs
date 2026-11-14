//! The session's background work: when the Agent says the session shown
//! is woken (its sub-agents reported while it ran no turn), the bridge
//! submits the woken turn with its own sink, so it shows like any turn,
//! opened by a "Background updates" cell. What the person asks meanwhile
//! waits for the submission's answer. Another session's wake is left to
//! that session's next turn. A wake that comes while a review runs, or that
//! is refused (a compaction runs), is tried again when that ends.

use hanafubuki_agent::{
    AgentBackgroundEvent, AgentBackgroundEventSink, AgentWakeState, AgentWakeTurnAccepted,
};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::{AppEvent, AppEventSender};

impl AgentBridge {
    /// What the bridge does with `event`.
    pub(crate) fn background(&mut self, event: AgentBackgroundEvent) {
        match event {
            AgentBackgroundEvent::Woken { session_id } if Some(session_id) == self.session_id => {
                self.wake()
            }
            AgentBackgroundEvent::Woken { .. } => {}
        }
    }

    /// Submits the session's woken turn. A turn that runs (or is being
    /// submitted) reads the events itself, and its end wakes the session
    /// again if any are left.
    fn wake(&mut self) {
        let Some(session_id) = self.session_id else {
            return;
        };
        if self.review_running() {
            self.missed_wake = true;
            return;
        }
        if self.turn_running || self.creating.is_some() || self.waking.is_some() {
            return;
        }
        self.turn_running = true;
        self.waking = Some(Vec::new());
        let sink = self.sink();
        self.spawn(move |agent| async move {
            let accepted = agent.submit_wake_turn_with_sink(session_id, sink).await;
            Ok(Some(AgentReply::WakeTurn(accepted)))
        });
    }

    /// The woken turn was submitted: its first cell lists what woke it, and
    /// what the person asked meanwhile is carried out.
    pub(super) fn woken(
        &mut self,
        accepted: Result<Option<AgentWakeTurnAccepted>, String>,
    ) -> Option<TurnUpdate> {
        let held = self.waking.take().unwrap_or_default();
        let update = match accepted {
            Ok(Some(woken)) => {
                self.turn = Some(woken.turn);
                Some(TurnUpdate::BackgroundUpdates(
                    woken
                        .events
                        .iter()
                        .map(|event| event.source.summary())
                        .collect(),
                ))
            }
            Ok(None) => {
                self.turn_running = false;
                None
            }
            // Refused while a compaction runs, say: tried again when one
            // ends.
            Err(error) => {
                self.turn_running = false;
                self.missed_wake = true;
                Some(TurnUpdate::Error(format!(
                    "the woken turn did not start: {error}"
                )))
            }
        };
        for op in held {
            if let Some(update) = self.submit(op) {
                self.app_event_tx
                    .send(AppEvent::AgentReply(AgentReply::Update(Box::new(update))));
            }
        }
        update
    }

    /// A wake that came while a review ran, or was refused, tried once the
    /// review or a compaction ended, unless the person paused the session's
    /// wakes meanwhile.
    pub(super) fn retry_missed_wake(&mut self) {
        if !std::mem::take(&mut self.missed_wake) {
            return;
        }
        let Some(session_id) = self.session_id else {
            return;
        };
        let on = self
            .agent
            .background_status(session_id)
            .is_ok_and(|status| status.wake == AgentWakeState::On);
        if on {
            self.wake();
        }
    }
}

/// The Agent's background events into the app loop: `event` must not
/// block.
pub(crate) struct BackgroundSink(pub(crate) AppEventSender);

impl AgentBackgroundEventSink for BackgroundSink {
    fn event(&self, event: AgentBackgroundEvent) {
        self.0.send(AppEvent::AgentBackground(event));
    }
}
