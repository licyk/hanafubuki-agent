//! A guest's observer, when its manifest declares events: the committed
//! events of the declared kinds are queued, at most [`QUEUE`] of them, and a
//! worker of the guest's fiber hands each to the guest's `observe`, one at a
//! time, without waiting for anything to come of it. An event the queue has
//! no room for is dropped.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    spawn_scoped_worker, AgentNotification, AgentObserver, CancellationToken, ObserverRegistry,
};
use kardia_core::Context;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{register_in, GuestCaller, GuestMethod};
use crate::package::GuestPackage;

/// The events one guest's queue holds.
const QUEUE: usize = 64;

/// The kinds a guest may observe, as its manifest names them.
pub(super) const EVENTS: [&str; 8] = [
    "turn_admitted",
    "sampling_committed",
    "turn_finished",
    "memory_contribution_changed",
    "session_archived",
    "session_restored",
    "wake_event_queued",
    "background_work_changed",
];

pub(super) fn register(
    ctx: &Context,
    guest: &GuestCaller,
    package: &GuestPackage,
    issues: &mut Vec<String>,
) {
    let declared = &package.contributes.observers;
    let (events, unknown): (Vec<&String>, Vec<&String>) = declared
        .iter()
        .partition(|event| EVENTS.contains(&event.as_str()));
    if !unknown.is_empty() {
        let unknown = unknown
            .iter()
            .map(|event| format!("`{event}`"))
            .collect::<Vec<_>>()
            .join(", ");
        issues.push(format!("it observes no events named {unknown}"));
    }
    if events.is_empty() {
        return;
    }
    let (queue, events_in) = mpsc::channel(QUEUE);
    let worker = guest.clone();
    let spawned = spawn_scoped_worker(ctx, "plugin-guest-observer", move |cancellation| {
        deliver(worker, events_in, cancellation)
    });
    if let Err(error) = spawned {
        issues.push(format!("its observer was not started: {error}"));
        return;
    }
    let observer = GuestObserver {
        id: format!("plugin:{}", package.id.key()),
        events: events.into_iter().cloned().collect(),
        queue,
    };
    register_in::<ObserverRegistry>(ctx, Arc::new(observer), "its observer", issues);
}

struct GuestObserver {
    id: String,
    events: Vec<String>,
    queue: mpsc::Sender<Value>,
}

impl AgentObserver for GuestObserver {
    fn id(&self) -> &str {
        &self.id
    }

    fn observe(&self, notification: &AgentNotification) {
        let event = event(notification);
        let declared = event["event"]
            .as_str()
            .is_some_and(|name| self.events.iter().any(|event| event == name));
        if declared && self.queue.try_send(event).is_err() {
            log::debug!("{}: an event was dropped: its queue is full", self.id);
        }
    }
}

async fn deliver(
    guest: GuestCaller,
    mut events: mpsc::Receiver<Value>,
    cancellation: CancellationToken,
) {
    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            () = super::cancelled(&cancellation) => None,
        };
        let Some(event) = event else {
            return;
        };
        if !guest.running() {
            continue;
        }
        let delivered = guest
            .call(GuestMethod::Observe, event, guest.deadline(), &cancellation)
            .await;
        if let Err(error) = delivered {
            guest.failure(GuestMethod::Observe, error);
        }
    }
}

/// The notification as the guest is told it.
fn event(notification: &AgentNotification) -> Value {
    match notification {
        AgentNotification::TurnAdmitted {
            session_id,
            turn_id,
            ..
        } => json!({
            "event": "turn_admitted",
            "sessionId": session_id.value(),
            "turnId": turn_id.value(),
        }),
        AgentNotification::SamplingCommitted {
            session_id,
            turn_id,
            ..
        } => json!({
            "event": "sampling_committed",
            "sessionId": session_id.value(),
            "turnId": turn_id.value(),
        }),
        AgentNotification::TurnFinished {
            session_id,
            turn_id,
            ..
        } => json!({
            "event": "turn_finished",
            "sessionId": session_id.value(),
            "turnId": turn_id.value(),
        }),
        AgentNotification::MemoryContributionChanged { session_id, .. } => json!({
            "event": "memory_contribution_changed",
            "sessionId": session_id.value(),
        }),
        AgentNotification::SessionArchived { session_id, .. } => json!({
            "event": "session_archived",
            "sessionId": session_id.value(),
        }),
        AgentNotification::SessionRestored { session_id, .. } => json!({
            "event": "session_restored",
            "sessionId": session_id.value(),
        }),
        AgentNotification::WakeEventQueued { session_id } => json!({
            "event": "wake_event_queued",
            "sessionId": session_id.value(),
        }),
        AgentNotification::BackgroundWorkChanged { session_id } => json!({
            "event": "background_work_changed",
            "sessionId": session_id.value(),
        }),
    }
}
