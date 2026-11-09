//! One row of a host's manifest and what the host keeps for it.

use std::sync::Arc;
use std::time::Duration;

use kardia_core::{Context, Fiber, PluginDef};
use serde_json::Value;
use tokio::sync::watch;

use super::bounds::{stop_stuck, watch_disposal, Teardown};
use super::restarts::PendingRestart;
use super::HostProfile;
use crate::restart::{RestartPolicy, RestartWindow};
use crate::status::{
    fiber_state, ModuleGroup, ModuleLock, ModuleRestart, ModuleState, ModuleStatus,
};
use crate::PluginError;

/// Derives a row's configuration from its host's settings.
pub type SettingsConfig<S> = fn(&S) -> Result<Value, String>;

/// One plugin to mount, its group, and where its configuration comes from.
pub struct ModuleRow<K: HostProfile> {
    pub(super) def: PluginDef,
    pub(super) group: K::Group,
    pub(super) from_settings: Option<SettingsConfig<K::Settings>>,
    /// What a service row's stop is given; the ordered shutdown waits that
    /// long for it, and its grace.
    pub(super) stop_bound: Option<Duration>,
}

impl<K: HostProfile> ModuleRow<K> {
    /// A plugin that takes no configuration.
    pub fn unconfigured(group: K::Group, def: PluginDef) -> Self {
        Self {
            def,
            group,
            from_settings: None,
            stop_bound: None,
        }
    }

    /// The services the row's plugin injects: it mounts once each of them
    /// is provided, and is mounted again when one of their providers is.
    pub fn inject(mut self, keys: impl Into<kardia_core::Inject>) -> Self {
        self.def = self.def.inject(keys);
        self
    }

    /// A plugin whose configuration follows the settings.
    pub fn from_settings(
        group: K::Group,
        def: PluginDef,
        config: SettingsConfig<K::Settings>,
    ) -> Self {
        Self {
            def,
            group,
            from_settings: Some(config),
            stop_bound: None,
        }
    }
}

pub(super) struct HostedRow<K: HostProfile> {
    pub(super) name: String,
    pub(super) group: K::Group,
    pub(super) def: PluginDef,
    pub(super) from_settings: Option<SettingsConfig<K::Settings>>,
    /// The configuration the row was (or will be) mounted or updated with.
    pub(super) config: Value,
    /// Whether the settings let it run.
    pub(super) enabled: bool,
    pub(super) mount: Mount,
    pub(super) stop_bound: Option<Duration>,
    /// The automatic restarts made in the current window.
    pub(super) restarts: RestartWindow,
    /// The automatic restart a fault scheduled, until it runs or is
    /// cancelled.
    pub(super) pending_restart: Option<PendingRestart>,
    /// What [`ModuleStatus::dependents`] reports; the manifest is fixed, so
    /// it is found once.
    pub(super) dependents: Vec<String>,
}

/// For each row, the rows that inject a service it provides and, in turn,
/// theirs, in manifest order.
pub(super) fn dependents_of<K: HostProfile>(rows: &[HostedRow<K>]) -> Vec<Vec<String>> {
    let injects = |dependent: &HostedRow<K>, provider: &HostedRow<K>| {
        let provided = provider.def.provide_spec();
        dependent
            .def
            .inject_spec()
            .keys()
            .any(|key| provided.contains(key))
    };
    (0..rows.len())
        .map(|provider| {
            let mut reached = vec![false; rows.len()];
            let mut pending = vec![provider];
            while let Some(current) = pending.pop() {
                for (index, row) in rows.iter().enumerate() {
                    if !reached[index] && index != provider && injects(row, &rows[current]) {
                        reached[index] = true;
                        pending.push(index);
                    }
                }
            }
            rows.iter()
                .zip(reached)
                .filter(|(_, reached)| *reached)
                .map(|(row, _)| row.name.clone())
                .collect()
        })
        .collect()
}

pub(super) enum Mount {
    NotStarted,
    Mounted(Fiber),
    Refused(String),
    /// Disposed because it was disabled, or failed (`fault`: it panicked,
    /// did not start in time, or its previous fiber did not stop in time),
    /// which it shows as `Failed` until a retry or a configuration change
    /// mounts it again. `stopped` reaches `Stopped` once its teardown, which
    /// removes its registrations, has ended; a new fiber of the row waits
    /// for that (bounded, `bounds.rs`), or its registrations would collide.
    Retired {
        stopped: watch::Receiver<Teardown>,
        fault: Option<String>,
    },
}

/// Whether the settings (and safe mode) let a row run; a locked row always
/// runs.
pub(super) fn enabled_in<K: HostProfile>(
    settings: &K::Settings,
    name: &str,
    group: K::Group,
    safe_mode: bool,
) -> bool {
    group.lock().is_some() || (!safe_mode && !K::disables(settings, name))
}

impl<K: HostProfile> HostedRow<K> {
    pub(super) fn lock(&self) -> Option<ModuleLock> {
        self.group.lock()
    }

    /// `issues` are what the row's callers left out because its code failed
    /// without stopping it; shown as the warning of an active row.
    /// `policy` is the host's automatic restart policy, if it has one.
    pub(super) fn status(
        &self,
        issues: Vec<String>,
        policy: Option<&RestartPolicy>,
    ) -> ModuleStatus<K::Group> {
        let stuck = matches!(
            &self.mount,
            Mount::Retired { stopped, .. } if *stopped.borrow() == Teardown::Stuck
        );
        let (state, error) = match &self.mount {
            _ if !self.enabled => (ModuleState::Disabled, stuck.then(stop_stuck)),
            Mount::Retired {
                fault: Some(fault), ..
            } if stuck => (
                ModuleState::Failed,
                Some(format!("{fault}; {}", stop_stuck())),
            ),
            Mount::Retired {
                fault: Some(fault), ..
            } => (ModuleState::Failed, Some(fault.clone())),
            Mount::Retired { .. } if stuck => (ModuleState::Failed, Some(stop_stuck())),
            Mount::NotStarted | Mount::Retired { .. } => (ModuleState::Pending, None),
            Mount::Refused(error) => (ModuleState::Failed, Some(error.clone())),
            Mount::Mounted(fiber) => fiber_state(fiber),
        };
        let warning =
            (state == ModuleState::Active && !issues.is_empty()).then(|| issues.join("; "));
        ModuleStatus {
            name: self.name.clone(),
            group: self.group,
            state,
            error,
            warning,
            enabled: self.enabled,
            lock: self.lock(),
            restart: policy.and_then(|policy| self.restart_status(policy)),
            dependents: self.dependents.clone(),
        }
    }

    fn restart_status(&self, policy: &RestartPolicy) -> Option<ModuleRestart> {
        let made = self.restarts.count(policy, std::time::Instant::now());
        let next = self
            .pending_restart
            .as_ref()
            .map(PendingRestart::at_unix_millis);
        (made > 0 || next.is_some()).then(|| ModuleRestart {
            restarts_in_window: u32::try_from(made).unwrap_or(u32::MAX),
            next_attempt_at_unix_millis: next,
        })
    }

    /// Cancels the automatic restart a fault scheduled, if one is pending:
    /// the row was turned off, retried, reconfigured or shut down.
    pub(super) fn cancel_restart(&mut self) {
        if let Some(pending) = self.pending_restart.take() {
            pending.cancel();
        }
    }

    /// Mounts the row on `root` with its current configuration.
    pub(super) fn mount(&mut self, root: &Context) -> Option<Fiber> {
        match root.plugin_def(&self.def, self.config.clone()) {
            Ok(fiber) => {
                self.mount = Mount::Mounted(fiber.clone());
                Some(fiber)
            }
            Err(error) => {
                self.mount = Mount::Refused(PluginError::from(error).to_string());
                None
            }
        }
    }

    /// Disposes a running fiber, because it was disabled or (with `fault`)
    /// because it failed. Its teardown continues on its own; the row
    /// records when it ended (or that it is stuck, past
    /// `PLUGIN_STOP_BOUND`), and `profile` hears of it, since its
    /// registrations are gone only then. Returns whether a fiber was
    /// disposed.
    pub(super) fn retire(&mut self, profile: Arc<K>, fault: Option<String>) -> bool {
        let Mount::Mounted(fiber) = &self.mount else {
            return false;
        };
        if fiber.is_removed() {
            return false;
        }
        let stopped = watch_disposal(self.name.clone(), fiber.dispose(), profile);
        self.mount = Mount::Retired { stopped, fault };
        true
    }

    /// Whether it was stopped because it failed.
    pub(super) fn is_faulted(&self) -> bool {
        matches!(self.mount, Mount::Retired { fault: Some(_), .. })
    }
}
