//! What a stop lost, reported to the surviving owners without waking them.

use std::collections::{BTreeMap, BTreeSet};

use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use hanafubuki_agent_kernel::{
    AgentInterruptedExecution, AgentSessionId, AgentSessionLifecycle, AgentSessionOrigin,
    AgentSubagentWake, AgentTerminalWake, AgentWakeSource,
};

use super::AgentApplicationService;

type LostWork = BTreeMap<AgentSessionId, Vec<AgentWakeSource>>;

#[cfg(test)]
mod tests;

impl AgentApplicationService {
    pub(super) fn capture_lost_background_work(&self) -> LostWork {
        let active = self
            .core
            .list_snapshots()
            .into_iter()
            .filter_map(|session| {
                session
                    .turn
                    .as_ref()
                    .is_some_and(|turn| !turn.phase.is_terminal())
                    .then_some(session.id)
            })
            .collect();
        self.lost_background_work(active, self.core.background_terminals_to_interrupt())
    }

    pub(super) fn lost_background_work(
        &self,
        active: BTreeSet<AgentSessionId>,
        terminals: Vec<AgentInterruptedExecution>,
    ) -> LostWork {
        let mut sessions = self
            .core
            .list_snapshots()
            .into_iter()
            .map(|session| (session.id, session))
            .collect::<BTreeMap<_, _>>();
        let mut lost_agents = BTreeSet::new();
        for id in active
            .into_iter()
            .chain(terminals.iter().map(|terminal| terminal.session_id))
        {
            let mut next = Some(id);
            while let Some(id) = next {
                // Recovery loads only sessions with unfinished operations;
                // the idle ancestors of a lost child are loaded lazily.
                if let std::collections::btree_map::Entry::Vacant(entry) = sessions.entry(id) {
                    if let Err(error) = self.core.activate_session(id) {
                        log::warn!("could not load background-work owner {id}: {error}");
                        break;
                    }
                    if let Ok(session) = self.core.snapshot(id) {
                        entry.insert(session);
                    }
                }
                let Some(session) = sessions.get(&id) else {
                    break;
                };
                if session.origin != Some(AgentSessionOrigin::Subagent)
                    || session.lifecycle == AgentSessionLifecycle::Archived
                    || !lost_agents.insert(id)
                {
                    break;
                }
                next = session.parent_session_id;
            }
        }
        let mut reports = LostWork::new();
        for id in &lost_agents {
            let session = &sessions[id];
            let Some(parent) = session
                .parent_session_id
                .filter(|parent| !lost_agents.contains(parent))
            else {
                continue;
            };
            reports
                .entry(parent)
                .or_default()
                .push(AgentWakeSource::Subagent {
                    session_id: *id,
                    name: session
                        .title
                        .as_ref()
                        .map(|title| title.value.clone())
                        .unwrap_or_else(|| format!("agent_{id}")),
                    event: AgentSubagentWake::Lost,
                });
        }
        for terminal in terminals {
            if !lost_agents.contains(&terminal.session_id) {
                reports
                    .entry(terminal.session_id)
                    .or_default()
                    .push(AgentWakeSource::Terminal {
                        execution_id: terminal.execution_id,
                        command: terminal.command,
                        event: AgentTerminalWake::Lost,
                    });
            }
        }
        reports
    }

    pub(super) fn report_lost_background_work(
        &self,
        lost: LostWork,
    ) -> Vec<WorkerShutdownDiagnostic> {
        lost.into_iter()
            .filter_map(|(owner, sources)| {
                self.core
                    .activate_session(owner)
                    .and_then(|()| self.core.record_lost_background_work(owner, sources))
                    .err()
                    .map(|error| WorkerShutdownDiagnostic {
                        worker: "agent-background-work".to_string(),
                        message: format!("could not record work lost by session {owner}: {error}"),
                    })
            })
            .collect()
    }
}
