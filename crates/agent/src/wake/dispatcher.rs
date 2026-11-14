//! The dispatcher: the core row `agent-wake`, whose observer hears of an
//! event queued on a session (`WakeEventQueued`) and of a turn's end
//! (`TurnFinished`, for events that arrived after its last step), and whose
//! worker then decides whether the session is woken. Every signal ends in
//! the same check of the session as it is now (its queue not empty, no turn
//! running, its policy on, the option on, its root not paused), and the
//! woken turn's admission is the kernel's: two signals for one session, or
//! a signal racing the person's message, wake it at most once. The observer
//! also clears a root's pause when the person's message is admitted.

use std::collections::BTreeSet;
use std::sync::Arc;

use hanafubuki_agent_contracts::{
    spawn_scoped_worker, AgentNotification, AgentObserver, AgentTurnOrigin, ObserverRegistry,
    PluginFaults, Register,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{AgentSessionId, AgentWakePolicy};
use kardia_core::{Context, Inject, PluginDef, Service};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::{AgentBackgroundEvent, Pauses, Wakes};
use crate::plugins::wait_for_cancellation;
use crate::session_driver::AgentSessionDriver;

/// The core row's plugin name.
const WAKE_PLUGIN: &str = "agent-wake";

/// Registers the dispatcher's observer and runs its worker over `driver`,
/// telling the host through `wakes`.
pub(crate) fn wake_plugin(driver: AgentSessionDriver, wakes: Wakes) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let driver = driver.clone();
        let wakes = wakes.clone();
        async move {
            let signals = Arc::new(Signals::default());
            let dispatcher = Dispatcher {
                driver,
                wakes: wakes.clone(),
                signals: signals.clone(),
            };
            spawn_scoped_worker(&ctx, "agent-wake", move |cancellation| {
                dispatcher.run(cancellation)
            })?;
            ctx.get::<ObserverRegistry>()?
                .register(Arc::new(WakeObserver {
                    signals,
                    pauses: wakes.pauses,
                }))?;
            Ok(())
        }
    })
    .name(WAKE_PLUGIN)
    .inject(
        Inject::new()
            .with(ObserverRegistry::NAME)
            .with(PluginFaults::NAME),
    )
}

/// The sessions signalled since the worker last looked: several signals
/// for one session are one look.
#[derive(Default)]
struct Signals {
    sessions: Mutex<BTreeSet<AgentSessionId>>,
    /// Keeps one permit while the worker is busy, so no signal is missed.
    ready: Notify,
}

impl Signals {
    fn signal(&self, session_id: AgentSessionId) {
        self.sessions.lock().insert(session_id);
        self.ready.notify_one();
    }

    fn take(&self) -> BTreeSet<AgentSessionId> {
        std::mem::take(&mut *self.sessions.lock())
    }
}

/// Bounded work only: it records a signal, or a root's pause.
struct WakeObserver {
    signals: Arc<Signals>,
    pauses: Pauses,
}

impl AgentObserver for WakeObserver {
    fn id(&self) -> &str {
        "wake"
    }

    fn observe(&self, notification: &AgentNotification) {
        match notification {
            AgentNotification::WakeEventQueued { session_id }
            | AgentNotification::TurnFinished { session_id, .. } => {
                self.signals.signal(*session_id)
            }
            // The person's message delivers what queued, and the session's
            // wakes resume.
            AgentNotification::TurnAdmitted {
                session_id,
                origin: AgentTurnOrigin::User,
                ..
            } => self.pauses.person_message(*session_id),
            _ => {}
        }
    }
}

struct Dispatcher {
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
                self.consider(session_id);
            }
        }
    }

    /// Wakes `session_id` if its queue holds events, no turn runs and
    /// nothing keeps it from being woken.
    fn consider(&self, session_id: AgentSessionId) {
        let Ok(session) = self.driver.loaded_snapshot(session_id) else {
            return;
        };
        let running = session
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal());
        if running
            || session.pending_wake_events == 0
            || session.wake_policy == AgentWakePolicy::Off
            || !self.driver.wakes_on_background_tasks()
        {
            return;
        }
        // Only a root is woken here, by its host: a sub-agent's events wait
        // for its next turn, and side conversations and reviews have the
        // policy off.
        if session.parent_session_id.is_some() || self.wakes.pauses.pause(session_id).is_some() {
            return;
        }
        self.wakes
            .sink
            .event(AgentBackgroundEvent::Woken { session_id });
    }
}
