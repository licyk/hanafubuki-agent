//! Following the settings, remounting and retrying rows.

use std::sync::Arc;

use kardia_core::{Context, FiberState, Settle};

use super::bounds::Teardown;
use super::row::{enabled_in, HostedRow, Mount};
use super::{HostProfile, HostRoot, ModuleHost};
use crate::PluginError;

impl<K: HostProfile> ModuleHost<K> {
    /// Brings every row up to date with the latest published settings: a row
    /// that became disabled is disposed, one that became enabled is mounted
    /// with its current configuration (after its previous fiber stopped), and
    /// an enabled running row whose derived configuration changed is updated;
    /// a row not running takes the new configuration for its next mount.
    /// Returns the transitions in flight; each tells the profile when it
    /// settles, and the host tells it at once when a row changed.
    ///
    /// The settings are read under the row lock, so of two concurrent calls
    /// the later one applies the later settings. Must run inside the Tokio
    /// runtime: kardia restarts the updated plugins on it.
    pub fn reconfigure(self: &Arc<Self>) -> Vec<(String, Settle)> {
        let mut state = self.state();
        let Some(settings) = self.profile.settings() else {
            return Vec::new();
        };
        let root = match &state.root {
            HostRoot::Running(root) => Some(root.clone()),
            HostRoot::NotStarted | HostRoot::Stopping(_) | HostRoot::Stopped => None,
        };
        let mut changed = false;
        let mut settles = Vec::new();
        for (index, row) in state.rows.iter_mut().enumerate() {
            let derived = row
                .from_settings
                .and_then(|from_settings| match from_settings(&settings) {
                    Ok(config) => Some(config),
                    Err(error) => {
                        log::error!(
                            "{} {} keeps its configuration; the settings do not derive one: {error}",
                            K::NOUN,
                            row.name
                        );
                        None
                    }
                });
            let enabled = enabled_in::<K>(&settings, &row.name, row.group, self.safe_mode);
            if enabled != row.enabled {
                row.enabled = enabled;
                changed = true;
                row.cancel_restart();
                if let Some(config) = derived {
                    row.config = config;
                }
                if !enabled {
                    row.retire(self.profile.clone(), None);
                } else if let Some(root) = &root {
                    if let Some(settle) = self.enable(root, index, row) {
                        settles.push((row.name.clone(), settle));
                    }
                }
                continue;
            }
            let Some(config) = derived.filter(|config| *config != row.config) else {
                continue;
            };
            // A new configuration mounts a row stopped by its fault again,
            // as it restarts a failed one.
            if row.enabled && row.is_faulted() {
                row.cancel_restart();
                row.config = config;
                changed = true;
                if let Some(root) = &root {
                    if let Some(settle) = self.enable(root, index, row) {
                        settles.push((row.name.clone(), settle));
                    }
                }
                continue;
            }
            match &row.mount {
                Mount::Mounted(fiber) if fiber.is_removed() => {}
                Mount::Mounted(fiber) if row.enabled => match fiber.update(config.clone()) {
                    Ok(settle) => {
                        row.config = config;
                        changed = true;
                        let settle = self.bounded(index, fiber.clone(), settle);
                        settles.push((row.name.clone(), settle));
                    }
                    Err(error) => log::error!(
                        "{} {} rejected its new configuration: {}",
                        K::NOUN,
                        row.name,
                        PluginError::from(error)
                    ),
                },
                _ => row.config = config,
            }
        }
        drop(state);
        if changed {
            self.profile.modules_changed();
        }
        settles
            .into_iter()
            .map(|(name, settle)| (name, self.notifying(settle)))
            .collect()
    }

    /// Mounts a row that became enabled. A row whose previous fiber is still
    /// stopping is mounted by a task once that fiber has stopped, if the row
    /// is still enabled and still waiting on that same fiber then. The wait
    /// is bounded by the disposal's own bound: a previous fiber reported
    /// stuck fails the row instead, until a retry after it stopped. The new
    /// fiber's start is bounded too.
    pub(super) fn enable(
        self: &Arc<Self>,
        root: &Context,
        index: usize,
        row: &mut HostedRow<K>,
    ) -> Option<Settle> {
        let retired = match &row.mount {
            Mount::Mounted(_) => return None,
            Mount::Retired { stopped, .. } if *stopped.borrow() != Teardown::Stopped => {
                stopped.clone()
            }
            Mount::NotStarted | Mount::Refused(_) | Mount::Retired { .. } => {
                let fiber = row.mount(root)?;
                let settle = fiber.settled();
                return Some(self.bounded(index, fiber, settle));
            }
        };
        let host = self.clone();
        let remount = self.spawn_task(async move {
            let mut stopped = retired.clone();
            // A sender gone without a value means the teardown task ended.
            let _ = stopped
                .wait_for(|teardown| *teardown != Teardown::Stopping)
                .await;
            let mut state = host.state();
            let HostRoot::Running(root) = &state.root else {
                return Ok(None);
            };
            let root = root.clone();
            let row = &mut state.rows[index];
            match &mut row.mount {
                Mount::Retired { stopped, fault }
                    if row.enabled && stopped.same_channel(&retired) =>
                {
                    if *stopped.borrow() == Teardown::Stopped {
                        // The new instance does not inherit the old one's
                        // warning.
                        host.faults.clear_issues(&row.name);
                        return Ok(row.mount(&root));
                    }
                    let stuck = "its previous instance has not stopped yet".to_string();
                    log::error!(
                        "{} {} is not mounted: {stuck}; retry it once it stopped",
                        K::NOUN,
                        row.name
                    );
                    fault.get_or_insert_with(|| stuck.clone());
                    Err(stuck)
                }
                _ => Ok(None),
            }
        });
        let host = self.clone();
        Some(Box::pin(async move {
            match remount.await {
                Ok(Some(Ok(Some(fiber)))) => {
                    let settle = fiber.settled();
                    host.bounded(index, fiber, settle).await
                }
                Ok(Some(Ok(None))) => Ok(()),
                Ok(Some(Err(stuck))) => Err(kardia_core::Error::msg(stuck)),
                Ok(None) => Err(kardia_core::Error::msg(format!(
                    "the {} host's remount panicked",
                    K::NOUN
                ))),
                Err(error) => Err(kardia_core::Error::new(error)),
            }
        }))
    }

    /// Tells the profile when `settle` ends, whichever way.
    pub(super) fn notifying(&self, settle: Settle) -> Settle {
        let profile = self.profile.clone();
        Box::pin(async move {
            let outcome = settle.await;
            profile.modules_changed();
            outcome
        })
    }

    /// Starts a `Failed` row again with its current configuration: one whose
    /// start failed is updated, one stopped because it failed (it faulted or
    /// timed out) is mounted again once its teardown has ended. Bounded like
    /// every start (`PLUGIN_START_TIMEOUT`).
    pub async fn retry(self: &Arc<Self>, name: &str) -> Result<(), String> {
        let settle = {
            let mut state = self.state();
            let root = match &state.root {
                HostRoot::Running(root) => Some(root.clone()),
                HostRoot::NotStarted | HostRoot::Stopping(_) | HostRoot::Stopped => None,
            };
            let index = state
                .rows
                .iter()
                .position(|row| row.name == name)
                .ok_or_else(|| format!("{} {name} does not exist", K::NOUN))?;
            let row = &mut state.rows[index];
            if !row.enabled {
                return Err(format!("{} {name} is disabled", K::NOUN));
            }
            // A retry is the user's: it takes the place of a pending
            // automatic restart and empties the restart window.
            row.cancel_restart();
            row.restarts.clear();
            match (&row.mount, root) {
                (Mount::Mounted(fiber), _) => {
                    if fiber.state() != FiberState::Failed {
                        return Err(format!("{} {name} has not failed", K::NOUN));
                    }
                    let settle = fiber
                        .update(row.config.clone())
                        .map_err(|error| PluginError::from(error).to_string())?;
                    self.bounded(index, fiber.clone(), settle)
                }
                (Mount::Retired { fault: Some(_), .. }, Some(root)) => self
                    .enable(&root, index, row)
                    .unwrap_or_else(|| Box::pin(async { Ok(()) })),
                _ => return Err(format!("{} {name} is not running", K::NOUN)),
            }
        };
        self.profile.modules_changed();
        self.notifying(settle)
            .await
            .map_err(|error| PluginError::from(error).to_string())
    }
}
