//! A person's confirmed terminal closes, projected and queued atomically.

use super::*;
use crate::{
    AgentClosedBackgroundTerminal, AgentExecutionProgressStatus, AgentTerminalWake,
    AgentToolProgressUpdate,
};

impl AgentService {
    /// Publishes confirmed process disposals and their close reports in one
    /// checkpoint, so a step cannot consume an exit between disposal and its
    /// replacement. Only model-started terminals wake their owner.
    pub fn report_closed_background_terminals(
        &self,
        session_id: AgentSessionId,
        terminals: Vec<AgentClosedBackgroundTerminal>,
    ) -> Result<(), AgentError> {
        if terminals.is_empty() {
            return Ok(());
        }
        let cell = self.session(session_id)?;
        self.flush_execution_progress_blocking(&cell);
        let (staged, events, woken) = {
            let mut record = cell.lock_after_persistence_blocking()?;
            if record.status.lifecycle == AgentSessionLifecycle::Archived {
                return Err(AgentError::SessionArchived);
            }
            if record.status.disposing {
                return Err(AgentError::SessionNotFound);
            }
            let previous = record.rollback_state();
            let projected = record.runtime.activities.active_snapshots();
            let mut events = Vec::new();
            let mut woken = false;
            for terminal in terminals {
                let model_started = record
                    .runtime
                    .background_terminals
                    .contains(terminal.execution_id, terminal.generation);
                let status = projected
                    .iter()
                    .filter_map(|activity| activity.execution.as_ref())
                    .find(|execution| {
                        execution.execution_id == terminal.execution_id
                            && execution.generation == terminal.generation
                    })
                    .map(|execution| execution.status)
                    .filter(|status| status.is_terminal())
                    .unwrap_or(AgentExecutionProgressStatus::Cancelled);
                for update in [
                    AgentToolProgressUpdate::ExecutionStatus {
                        execution_id: terminal.execution_id,
                        generation: terminal.generation,
                        status,
                        exit_code: terminal.exit_code,
                        cleanup_required: false,
                    },
                    AgentToolProgressUpdate::ExecutionDisposed {
                        execution_id: terminal.execution_id,
                        generation: terminal.generation,
                    },
                ] {
                    let event = record_session_event(
                        &mut record,
                        AgentEventKind::ExecutionProgress {
                            call_id: None,
                            update: update.clone(),
                        },
                        self.inner.limits,
                    );
                    track_background_terminal(
                        &mut record,
                        false,
                        &update,
                        event.activity.as_ref(),
                        None,
                    );
                    events.push(event);
                }
                if model_started && record.runtime.wake_policy == AgentWakePolicy::On {
                    record.runtime.wake_queue.push(
                        AgentWakeSource::Terminal {
                            execution_id: terminal.execution_id,
                            command: terminal.command,
                            event: AgentTerminalWake::StoppedByUser {
                                exit_code: terminal.exit_code,
                            },
                        },
                        now_millis_u64(),
                    );
                    woken = true;
                }
            }
            (
                StagedPersistence::commit(&cell, &mut record, previous),
                events,
                woken,
            )
        };
        self.persist_staged_blocking(staged)?;
        for event in events {
            self.emit_session_event(session_id, event);
        }
        if woken {
            self.inner
                .registries
                .observers
                .notify(&AgentNotification::WakeEventQueued { session_id });
        } else {
            self.notify_background_work_changed(session_id);
        }
        Ok(())
    }
}
