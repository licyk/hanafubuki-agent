//! The dispatcher: the core row `agent-wake`, whose observer hears of an
//! event queued on a session (`WakeEventQueued`) and of a turn's end
//! (`TurnFinished`, for events that arrived after its last step), and whose
//! worker then decides whether the session is woken. Every signal ends in
//! the same check of the session as it is now (its queue not empty, no turn
//! running, its policy on, the option on, its root not paused), and the
//! woken turn's admission is the kernel's: two signals for one session, or
//! a signal racing the person's message, wake it at most once. The observer
//! also clears a root's pause when the person's message is admitted, and
//! hints the feed (`feed.rs`) of what may change a tree, which the row's
//! second worker reads.

use std::collections::BTreeSet;
use std::sync::Arc;

use hanafubuki_agent_contracts::{
    spawn_scoped_worker, AgentNotification, AgentObserver, AgentTurnOrigin, BlockingService,
    ObserverRegistry, PluginFaults, Register,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_host::FaultPoint;
use hanafubuki_agent_kernel::{AgentSessionId, AgentSessionOrigin, AgentWakePolicy};
use kardia_core::{Context, Inject, PluginDef, Service};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::feed::{Feed, FeedWorker};
use super::{AgentBackgroundEvent, Pauses, Wakes};
use crate::plugins::{wait_for_cancellation, SubagentService, SUBAGENTS_PLUGIN};
use crate::session_driver::AgentSessionDriver;

/// The core row's plugin name.
const WAKE_PLUGIN: &str = "agent-wake";

/// Registers the dispatcher's observer and runs its worker and the feed's
/// over `driver`, telling the host through `wakes`.
pub(crate) fn wake_plugin(driver: AgentSessionDriver, wakes: Wakes) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let driver = driver.clone();
        let wakes = wakes.clone();
        async move {
            let signals = wakes.signals.clone();
            let dispatcher = Dispatcher {
                driver: driver.clone(),
                context: ctx.clone(),
                blocking: ctx.get::<BlockingService>()?.arc().clone(),
                faults: ctx.get::<PluginFaults>()?.arc().clone(),
                wakes: wakes.clone(),
                signals: signals.clone(),
            };
            spawn_scoped_worker(&ctx, "agent-wake", move |cancellation| {
                dispatcher.run(cancellation)
            })?;
            let feed = FeedWorker {
                driver,
                wakes: wakes.clone(),
                blocking: ctx.get::<BlockingService>()?.arc().clone(),
            };
            spawn_scoped_worker(&ctx, "agent-wake-feed", move |cancellation| {
                feed.run(cancellation)
            })?;
            ctx.get::<ObserverRegistry>()?
                .register(Arc::new(WakeObserver {
                    signals,
                    pauses: wakes.pauses,
                    feed: wakes.feed,
                }))?;
            Ok(())
        }
    })
    .name(WAKE_PLUGIN)
    .inject(
        Inject::new()
            .with(ObserverRegistry::NAME)
            .with(PluginFaults::NAME)
            .with(BlockingService::NAME),
    )
}

/// The sessions signalled since the worker last looked: several signals
/// for one session are one look.
#[derive(Default)]
pub(crate) struct Signals {
    sessions: Mutex<BTreeSet<AgentSessionId>>,
    /// Keeps one permit while the worker is busy, so no signal is missed.
    ready: Notify,
}

impl Signals {
    pub(crate) fn signal(&self, session_id: AgentSessionId) {
        self.sessions.lock().insert(session_id);
        self.ready.notify_one();
    }

    fn take(&self) -> BTreeSet<AgentSessionId> {
        std::mem::take(&mut *self.sessions.lock())
    }
}

/// Bounded work only: it records a signal, a hint, or a root's pause.
struct WakeObserver {
    signals: Arc<Signals>,
    pauses: Pauses,
    feed: Arc<Feed>,
}

impl AgentObserver for WakeObserver {
    fn id(&self) -> &str {
        "wake"
    }

    fn observe(&self, notification: &AgentNotification) {
        match notification {
            AgentNotification::WakeEventQueued { session_id }
            | AgentNotification::TurnFinished { session_id, .. } => {
                self.signals.signal(*session_id);
                self.feed.hint(*session_id);
            }
            AgentNotification::TurnAdmitted {
                session_id, origin, ..
            } => {
                // The person's message delivers what queued, and the
                // session's wakes resume.
                if *origin == AgentTurnOrigin::User {
                    self.pauses.person_message(*session_id);
                }
                self.feed.hint(*session_id);
                if *origin == AgentTurnOrigin::User {
                    self.signals.signal(*session_id);
                }
            }
            AgentNotification::BackgroundWorkChanged { session_id }
            | AgentNotification::SessionArchived { session_id, .. }
            | AgentNotification::SessionRestored { session_id, .. } => {
                self.feed.hint(*session_id);
                self.signals.signal(*session_id);
            }
            AgentNotification::SamplingCommitted { .. }
            | AgentNotification::MemoryContributionChanged { .. } => {}
        }
    }
}

struct Dispatcher {
    context: Context,
    blocking: Arc<BlockingService>,
    faults: Arc<PluginFaults>,
    driver: AgentSessionDriver,
    wakes: Wakes,
    signals: Arc<Signals>,
}

impl Dispatcher {
    async fn run(self, cancellation: CancellationToken) {
        loop {
            tokio::select! {
                () = self.signals.ready.notified() => {}
                () = wait_for_cancellation(cancellation.clone()) => return,
            }
            for session_id in self.signals.take() {
                self.consider(session_id).await;
            }
        }
    }

    /// Wakes `session_id` if its queue holds events, no turn runs and
    /// nothing keeps it from being woken.
    async fn consider(&self, session_id: AgentSessionId) {
        let Ok(session) = self.driver.loaded_snapshot(session_id) else {
            return;
        };
        let (driver, wakes) = (self.driver.clone(), self.wakes.clone());
        let Ok(Ok((root, descendants))) = self
            .blocking
            .run("agent wake lineage", move || {
                let root = super::root_of(&driver, session_id)?;
                let descendants = if root == session_id {
                    wakes
                        .runs
                        .tree_statuses(&driver, root, false)?
                        .into_iter()
                        .map(|agent| agent.session_id)
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                Ok::<_, hanafubuki_agent_kernel::AgentError>((root, descendants))
            })
            .await
        else {
            return;
        };
        for child in descendants {
            self.signals.signal(child);
        }
        let service = self.context.live_as::<SubagentService>(SUBAGENTS_PLUGIN);
        if let Some(service) = &service {
            let service = service.arc().clone();
            let _ = self
                .faults
                .guard_async(
                    SUBAGENTS_PLUGIN,
                    FaultPoint::new("settle background work"),
                    || async {
                        service.settle(session_id).await;
                        if let Some(parent) = session.parent_session_id {
                            service.settle(parent).await;
                        }
                    },
                )
                .await;
        }
        let Ok(session) = self.driver.loaded_snapshot(session_id) else {
            return;
        };
        if session
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
            || session.pending_wake_events == 0
            || session.wake_policy == AgentWakePolicy::Off
            || !self.driver.wakes_on_background_tasks()
            || self.wakes.pauses.pause(root).is_some()
        {
            return;
        }
        if session.origin == Some(AgentSessionOrigin::Subagent) {
            if let Some(service) = service {
                let service = service.arc().clone();
                let result = self
                    .faults
                    .guard_async(
                        SUBAGENTS_PLUGIN,
                        FaultPoint::new("resume background work"),
                        || service.resume(session_id),
                    )
                    .await;
                if let Ok(Err(error)) = result {
                    log::debug!("subagent {session_id} was not resumed: {error}");
                }
            }
        } else if session.parent_session_id.is_none() {
            self.wakes
                .sink
                .event(AgentBackgroundEvent::Woken { session_id });
        }
    }
}
