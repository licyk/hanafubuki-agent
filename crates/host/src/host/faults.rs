//! Stopping a row whose code faulted.
//!
//! Two sources report a fault of a row. The host's [`crate::PluginFaults`]
//! gets a panic in the row's plugin code (a registry entry's guard, a scoped
//! worker). A host whose rows are application modules
//! ([`super::ModuleHostOptions::module_faults`]) also subscribes to the
//! application's module fault sink
//! (`hanafubuki_agent_foundation::module_faults`), which gets a panic in a
//! module's contained worker groups and threads and in its commands, by
//! module name. Either report arrives on whatever thread caught the panic,
//! so the host only queues it; a host task on the Tokio runtime, where
//! kardia mutations must run, disposes the row. The row then shows `Failed`
//! with the fault until a retry or a new configuration mounts it again.
//!
//! A row's issues (what its callers left out without stopping it) are its
//! `warning`; the host announces each change to them as a row change, so the
//! status does not wait for the next read.

use std::collections::BTreeSet;
use std::sync::{Arc, Weak};

use hanafubuki_agent_foundation::module_faults::{subscribe_module_faults, FaultOwner};
use tokio::sync::mpsc;

use super::{HostProfile, ModuleHost};
use crate::status::ModuleGroup;

/// A fault of the row named `module`; `reason` is what its status shows.
struct RowFault {
    module: String,
    reason: String,
}

impl<K: HostProfile> ModuleHost<K> {
    /// Starts handling the faults of the rows, and announcing their issue
    /// changes. Runs inside the Tokio runtime, once, when the host starts;
    /// shutdown stops both.
    pub(super) fn follow_faults(self: &Arc<Self>) {
        let profile = self.profile.clone();
        self.faults
            .set_issues_listener(move || profile.modules_changed());
        let (sender, mut faults) = mpsc::unbounded_channel();
        if self.module_faults {
            let names = self
                .state()
                .rows
                .iter()
                .map(|row| row.name.clone())
                .collect::<BTreeSet<_>>();
            let sender = sender.clone();
            let subscription = subscribe_module_faults(move |fault| {
                let FaultOwner::Module(module) = fault.owner else {
                    return;
                };
                if names.contains(module) {
                    let _ = sender.send(RowFault {
                        module: module.to_string(),
                        reason: format!("panicked in {}: {}", fault.component, fault.message),
                    });
                }
            });
            self.state().subscription = Some(subscription);
        }
        self.faults.set_handler(move |fault| {
            let _ = sender.send(RowFault {
                module: fault.plugin.clone(),
                reason: fault.to_string(),
            });
        });
        let host: Weak<Self> = Arc::downgrade(self);
        self.spawn_task(async move {
            while let Some(fault) = faults.recv().await {
                let Some(host) = host.upgrade() else {
                    return;
                };
                host.stop_faulted(fault);
            }
        });
    }

    /// Disposes the running row `fault` names and records the fault. A row
    /// already stopped, a core row and an unknown name only stay in the log,
    /// where the fault already is.
    fn stop_faulted(self: &Arc<Self>, fault: RowFault) {
        let stopped = {
            let mut state = self.state();
            match state
                .rows
                .iter_mut()
                .enumerate()
                .find(|(_, row)| row.name == fault.module)
            {
                Some((index, row)) if !row.group.is_core() => row
                    .retire(self.profile.clone(), Some(fault.reason.clone()))
                    .then(|| self.schedule_restart(index, row)),
                Some(_) | None => None,
            }
        };
        let Some(restart) = stopped else {
            return;
        };
        match restart {
            Some(delay) => log::error!(
                "{} {} is stopped because it {}; it starts again in {} ms",
                K::NOUN,
                fault.module,
                fault.reason,
                delay.as_millis()
            ),
            None => log::error!(
                "{} {} is stopped because it {}; retry it to start it again",
                K::NOUN,
                fault.module,
                fault.reason
            ),
        }
        self.profile.modules_changed();
    }
}
