//! The feed: the host is told [`AgentBackgroundEvent::Changed`] for a root
//! when what `background_tasks(root)` answers changed. What may change it
//! hints the session it touched ([`Feed::hint`]): the dispatcher's observer
//! (a turn admitted or finished, events queued on or leaving a queue, a
//! session archived or restored), a sub-agent's run as it starts and ends,
//! the facade (a pause, a woken turn counted, a session's model, effort or
//! title, the settings). The worker `agent-wake-feed` maps each hinted
//! session to its root, reads the root's tree without the agents' last
//! answers, and tells the host only of a root whose tree differs from what
//! it read last; hints that arrive together are one look. A sub-agent that
//! streams hints nothing, and a turn's steps hint nothing.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::AgentSessionId;
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::tasks::root_of;
use super::{AgentBackgroundEvent, AgentBackgroundTasks, Wakes};
use crate::plugins::wait_for_cancellation;
use crate::session_driver::AgentSessionDriver;

/// The sessions hinted since the worker last looked.
#[derive(Default)]
pub(crate) struct Feed {
    hints: Mutex<Hints>,
    /// Keeps one permit while the worker is busy, so no hint is missed.
    ready: Notify,
}

#[derive(Default)]
struct Hints {
    sessions: BTreeSet<AgentSessionId>,
    /// Every root the worker knows is looked at again.
    all: bool,
}

impl Feed {
    /// What `session_id`'s tree holds may have changed.
    pub(crate) fn hint(&self, session_id: AgentSessionId) {
        self.hints.lock().sessions.insert(session_id);
        self.ready.notify_one();
    }

    /// What every tree holds may have changed (`wakeOnBackgroundTasks`).
    pub(crate) fn hint_all(&self) {
        self.hints.lock().all = true;
        self.ready.notify_one();
    }

    fn take(&self) -> Hints {
        std::mem::take(&mut *self.hints.lock())
    }
}

/// The worker: what it last read of each root it looked at.
pub(super) struct FeedWorker {
    pub(super) driver: AgentSessionDriver,
    pub(super) wakes: Wakes,
    pub(super) blocking: Arc<BlockingService>,
}

impl FeedWorker {
    pub(super) async fn run(self, cancellation: CancellationToken) {
        let mut read: HashMap<AgentSessionId, AgentBackgroundTasks> = HashMap::new();
        loop {
            tokio::select! {
                () = self.wakes.feed.ready.notified() => {}
                () = wait_for_cancellation(cancellation.clone()) => return,
            }
            let hints = self.wakes.feed.take();
            let mut sessions = hints.sessions;
            if hints.all {
                sessions.extend(read.keys());
            }
            let (driver, wakes) = (self.driver.clone(), self.wakes.clone());
            let trees = self
                .blocking
                .run("agent background feed", move || {
                    read_trees(&driver, &wakes, sessions)
                })
                .await;
            let Ok(trees) = trees else {
                continue;
            };
            for (root, tasks) in trees {
                let idle = AgentBackgroundTasks::idle();
                if read.get(&root).unwrap_or(&idle) == &tasks {
                    continue;
                }
                read.insert(root, tasks);
                self.wakes
                    .sink
                    .event(AgentBackgroundEvent::Changed { root });
            }
        }
    }
}

/// The tree of each hinted session's root, without the agents' answers;
/// a root that cannot be read is left out. Blocks.
fn read_trees(
    driver: &AgentSessionDriver,
    wakes: &Wakes,
    sessions: BTreeSet<AgentSessionId>,
) -> Vec<(AgentSessionId, AgentBackgroundTasks)> {
    let roots = sessions
        .into_iter()
        .filter_map(|session| root_of(driver, session).ok())
        .collect::<BTreeSet<_>>();
    roots
        .into_iter()
        .filter_map(|root| match wakes.tasks(driver, root, false) {
            Ok(tasks) => Some((root, tasks)),
            Err(error) => {
                log::debug!("the background of session {root} was not read: {error}");
                None
            }
        })
        .collect()
}
