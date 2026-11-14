//! Wake events queued by hand, a session's policy given by hand, and a host
//! that records whom the Agent tells it to wake.

use std::sync::Arc;

use hanafubuki_agent_kernel::{
    AgentError, AgentEvent, AgentSessionId, AgentWakePolicy, AgentWakeQueued, AgentWakeSource,
};
use hanafubuki_agent_process::executor::ExecutorManager;
use parking_lot::Mutex;

use super::TestPaths;
use crate::{
    AgentApplicationService, AgentBackgroundEvent, AgentBackgroundEventSink, AgentConfig,
    AgentHostPorts, AgentStoragePaths, SkillRoots,
};

/// What a host's background sink was told, in order; it submits nothing.
#[derive(Clone, Default)]
pub struct RecordedBackgroundEvents(Arc<Mutex<Vec<AgentBackgroundEvent>>>);

impl RecordedBackgroundEvents {
    pub fn events(&self) -> Vec<AgentBackgroundEvent> {
        self.0.lock().clone()
    }

    /// How many times `session_id` was woken.
    pub fn woken(&self, session_id: AgentSessionId) -> usize {
        self.0
            .lock()
            .iter()
            .filter(|event| **event == AgentBackgroundEvent::Woken { session_id })
            .count()
    }

    /// How many times the host was told that `root`'s tree changed.
    pub fn changed(&self, root: AgentSessionId) -> usize {
        self.0
            .lock()
            .iter()
            .filter(|event| **event == AgentBackgroundEvent::Changed { root })
            .count()
    }

    /// The events of sub-agents' turns told for `root`'s tree, in order.
    pub fn child_events(&self, root: AgentSessionId) -> Vec<AgentEvent> {
        self.0
            .lock()
            .iter()
            .filter_map(|event| match event {
                AgentBackgroundEvent::Child {
                    root: told, event, ..
                } if *told == root => Some((**event).clone()),
                _ => None,
            })
            .collect()
    }
}

impl AgentBackgroundEventSink for RecordedBackgroundEvents {
    fn event(&self, event: AgentBackgroundEvent) {
        self.0.lock().push(event);
    }
}

/// An Agent over `paths`' storage, as `reopen_service` builds it, that
/// tells `background` which sessions to wake.
pub fn reopen_service_waking(
    config: AgentConfig,
    host: AgentHostPorts,
    paths: &TestPaths,
    background: Arc<dyn AgentBackgroundEventSink>,
) -> AgentApplicationService {
    AgentApplicationService::new_with_host_discovering(
        config,
        host,
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
        crate::model_discovery::ModelDiscoveryPolicy::OFF,
        background,
    )
    .unwrap()
}

/// Gives `session_id` the wake `policy` (the kernel's default is off);
/// whether it changed.
pub async fn set_wake_policy(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    policy: AgentWakePolicy,
) -> Result<bool, AgentError> {
    service.core.activate_session(session_id)?;
    service
        .core
        .set_session_wake_policy(session_id, policy)
        .await
}

/// Queues `source` on `session_id`, as a sub-agent's end or a terminal's
/// would be.
pub async fn queue_wake_event(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    source: AgentWakeSource,
) -> Result<AgentWakeQueued, AgentError> {
    service.core.activate_session(session_id)?;
    service.core.enqueue_wake_event(session_id, source).await
}
