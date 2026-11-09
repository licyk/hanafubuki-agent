//! A kardia host for one manifest of rows
//! (`site/content/docs/plugin-rules.en.mdx`).
//!
//! The host owns a kardia root and mounts every row of its manifest on it,
//! in order, keeping one fiber per row. kardia needs the ambient Tokio
//! runtime, so the host only starts from async code, and runs on the
//! runtime of whoever starts it. It follows its settings
//! ([`ModuleHost::reconfigure`]): a row whose configuration is derived from
//! them is updated when that configuration changes, a row they disable is
//! disposed (its registrations leave with it) and mounted again when they
//! enable it. Core rows cannot be disabled. A row whose code faults is
//! stopped and `Failed` until it is retried (`faults.rs`), and every wait on
//! kardia is bounded (`bounds.rs`).
//!
//! What varies between hosts is a [`HostProfile`]: the groups of its rows,
//! the settings they follow, where it reports a change and its own panics,
//! and how it names its rows. The Agent's plugin host is one.

mod bounds;
mod faults;
mod lookup;
mod reconfigure;
mod restarts;
mod row;
mod service_row;

use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use hanafubuki_agent_foundation::module_faults::ModuleFaultSubscription;
use hanafubuki_agent_foundation::panic::catch_panic_async;
use kardia_core::{Context, Fiber, Settle};
use parking_lot::{Mutex, MutexGuard};
use serde_json::Value;
use tokio::sync::OnceCell;
use tokio::task::JoinHandle;

pub use bounds::{PLUGIN_START_TIMEOUT, PLUGIN_STOP_BOUND};
use row::{dependents_of, enabled_in, HostedRow, Mount};
pub use row::{ModuleRow, SettingsConfig};
pub use service_row::{ServiceContext, ServiceMount, ServiceStop};

use crate::restart::{RestartPolicy, RestartWindow};
use crate::status::{ModuleGroup, ModuleState, ModuleStatus};
use crate::{LogExporter, PluginError, PluginFaults};

/// What one host is: the groups its rows declare, the settings their
/// configuration and enablement follow, where it reports, and how it names
/// what it runs.
pub trait HostProfile: Send + Sync + Sized + 'static {
    type Group: ModuleGroup;
    type Settings: Send + Sync + 'static;

    /// What a row is called in logs and errors (`Agent plugin`).
    const NOUN: &'static str;
    /// What runs the rows, in logs and errors (`the Agent`).
    const HOST: &'static str;
    /// The worker named in the shutdown diagnostic (`agent-plugins`).
    const SHUTDOWN_WORKER: &'static str;

    /// The latest published settings; `None` before they are loaded.
    fn settings(&self) -> Option<Self::Settings>;

    /// Whether `settings` turn off the row named `name`.
    fn disables(settings: &Self::Settings, name: &str) -> bool;

    /// A row may have changed: it was enabled, disabled, retried, faulted or
    /// reconfigured, such a change settled, a teardown ended or got stuck,
    /// or its issues changed. It carries nothing; readers read the status
    /// again. It may be called on any thread and must not block.
    fn modules_changed(&self);

    /// One of the host's own tasks (the fault follower, a remount)
    /// panicked with `message`; the panic is already logged.
    fn host_task_panicked(&self, message: &str);

    /// Provides on the host's root, before any row mounts, the services the
    /// host itself owns for its rows (a registry they contribute to). They
    /// stay for the host's lifetime, whichever rows run.
    fn provide_root(&self, root: &RootServices<'_>) -> Result<(), String> {
        let _ = root;
        Ok(())
    }
}

/// The host's root, where [`HostProfile::provide_root`] provides the
/// services the host owns for its rows.
pub struct RootServices<'a> {
    root: &'a Context,
}

impl RootServices<'_> {
    /// Provides `service` under its name to every row that injects it.
    pub fn provide<S: kardia_core::Service>(&self, service: Arc<S>) -> Result<(), String> {
        self.root
            .provide_arc(service)
            .map(|_| ())
            .map_err(|error| PluginError::from(error).to_string())
    }
}

/// How one host runs.
#[derive(Clone, Debug, Default)]
pub struct ModuleHostOptions {
    /// Every row outside the core stays unmounted, whatever the settings
    /// say; the saved settings are not changed.
    pub safe_mode: bool,
    /// The rows are application modules: a fault the application's module
    /// fault sink records for one of their names stops that row. A host of
    /// plugins inside one module (the Agent's) leaves it off, since its row
    /// names are not module names.
    pub module_faults: bool,
    /// A row stopped by a fault is mounted again by itself after the
    /// policy's backoff, while its window allows (`restarts.rs`). Without
    /// one a faulted row stays `Failed` until it is retried.
    pub automatic_restarts: Option<RestartPolicy>,
}

pub struct ModuleHost<K: HostProfile> {
    profile: Arc<K>,
    /// Where the rows' plugin code reports its panics and issues.
    faults: Arc<PluginFaults>,
    safe_mode: bool,
    module_faults: bool,
    restart_policy: Option<RestartPolicy>,
    /// Tells apart the automatic restarts scheduled.
    next_restart: AtomicU64,
    state: Mutex<HostState<K>>,
    started: OnceCell<()>,
}

struct HostState<K: HostProfile> {
    root: HostRoot,
    rows: Vec<HostedRow<K>>,
    /// Held while the host follows the module fault sink.
    subscription: Option<ModuleFaultSubscription>,
}

enum HostRoot {
    NotStarted,
    /// Set when start-up begins, so a reconfiguration during start-up mounts
    /// what it enables.
    Running(Context),
    /// The ordered shutdown is stopping the rows one by one: a row it has
    /// not reached still serves, and nothing is mounted again.
    Stopping(Context),
    /// Shut down; nothing is mounted again.
    Stopped,
}

impl<K: HostProfile> ModuleHost<K> {
    /// Derives every row's configuration and enablement from the published
    /// settings, which the caller has loaded. `faults` is where the rows'
    /// plugin code reports (a row that injects it gets it from a row that
    /// provides it).
    pub fn new(
        profile: K,
        faults: Arc<PluginFaults>,
        rows: Vec<ModuleRow<K>>,
        options: ModuleHostOptions,
    ) -> Result<Self, String> {
        let current = profile
            .settings()
            .ok_or_else(|| "settings are not loaded".to_string())?;
        let mut rows = rows
            .into_iter()
            .map(|row| {
                let name = row.def.plugin_name().unwrap_or("anonymous").to_string();
                Ok(HostedRow {
                    enabled: enabled_in::<K>(&current, &name, row.group, options.safe_mode),
                    name,
                    group: row.group,
                    config: match row.from_settings {
                        Some(from_settings) => from_settings(&current)?,
                        None => Value::Null,
                    },
                    def: row.def,
                    from_settings: row.from_settings,
                    mount: Mount::NotStarted,
                    stop_bound: row.stop_bound,
                    restarts: RestartWindow::default(),
                    pending_restart: None,
                    dependents: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let dependents = dependents_of(&rows);
        for (row, dependents) in rows.iter_mut().zip(dependents) {
            row.dependents = dependents;
        }
        Ok(Self {
            profile: Arc::new(profile),
            faults,
            safe_mode: options.safe_mode,
            module_faults: options.module_faults,
            restart_policy: options.automatic_restarts,
            next_restart: AtomicU64::new(0),
            state: Mutex::new(HostState {
                root: HostRoot::NotStarted,
                rows,
                subscription: None,
            }),
            started: OnceCell::new(),
        })
    }

    /// Mounts the enabled rows once, and from then on stops a row whose
    /// code faults (`faults.rs`). A row that fails to start is logged and
    /// left out; the host runs without it.
    pub async fn ensure_started(self: &Arc<Self>) {
        self.started
            .get_or_init(|| async {
                self.follow_faults();
                self.start().await
            })
            .await;
    }

    /// Mounts the rows in order, each after the previous one settled or
    /// timed out (`PLUGIN_START_TIMEOUT`: the row fails, its fiber is
    /// disposed in the background, and the next row mounts), and then
    /// reports every enabled row that is not active as a start-up
    /// diagnostic.
    async fn start(self: &Arc<Self>) {
        let root = Context::new();
        if let Err(error) = root.add_exporter(Arc::new(LogExporter)) {
            log::warn!(
                "{} logs are unavailable: {}",
                K::NOUN,
                PluginError::from(error)
            );
        }
        if let Err(error) = self.profile.provide_root(&RootServices { root: &root }) {
            log::error!("{} did not provide its own services: {error}", K::HOST);
        }
        let count = {
            let mut state = self.state();
            if !matches!(state.root, HostRoot::NotStarted) {
                return;
            }
            state.root = HostRoot::Running(root);
            state.rows.len()
        };
        for index in 0..count {
            if let Some(fiber) = self.mount_at_start(index) {
                let settle = fiber.settled();
                let _ = self.bounded(index, fiber, settle).await;
            }
        }
        for status in self.status() {
            match (status.state, status.error) {
                (ModuleState::Active | ModuleState::Disabled, _) => {}
                (state, Some(error)) => log::error!(
                    "{} {} is not active after start-up ({state:?}): {error}",
                    K::NOUN,
                    status.name
                ),
                (state, None) => log::error!(
                    "{} {} is not active after start-up ({state:?})",
                    K::NOUN,
                    status.name
                ),
            }
        }
        self.profile.modules_changed();
    }

    /// Mounts an enabled row that is not mounted yet, under the row lock,
    /// so a concurrent [`Self::reconfigure`] either changes what this mount
    /// uses or acts on its fiber.
    fn mount_at_start(&self, index: usize) -> Option<Fiber> {
        let mut state = self.state();
        let HostRoot::Running(root) = &state.root else {
            return None;
        };
        let root = root.clone();
        let row = &mut state.rows[index];
        if !row.enabled || !matches!(row.mount, Mount::NotStarted) {
            return None;
        }
        row.mount(&root)
    }

    /// Every row's state, in mount order, with the issues of the active
    /// ones as their warning.
    pub fn status(&self) -> Vec<ModuleStatus<K::Group>> {
        let mut issues = self.faults.all_issues();
        self.state()
            .rows
            .iter()
            .map(|row| {
                row.status(
                    issues.remove(&row.name).unwrap_or_default(),
                    self.restart_policy.as_ref(),
                )
            })
            .collect()
    }

    /// Checks that `name` is a row whose enablement may become `enabled`.
    pub fn check_enablement(&self, name: &str, enabled: bool) -> Result<(), String> {
        let state = self.state();
        let row = state
            .rows
            .iter()
            .find(|row| row.name == name)
            .ok_or_else(|| format!("{} {name} does not exist", K::NOUN))?;
        if !enabled && row.lock().is_some() {
            return Err(format!("{} {name} cannot be disabled", K::NOUN));
        }
        if enabled && self.safe_mode && row.lock().is_none() {
            return Err(format!(
                "{} runs in safe mode, which keeps {name} off; restart it normally first",
                K::HOST
            ));
        }
        Ok(())
    }

    /// Stops every row. Their registrations are reverted after running
    /// calls drain (bounded by the registry's drain grace), and their
    /// workers stop with them. A row that does not stop by `deadline` is
    /// left stopping and reported. Nothing is mounted afterwards.
    pub async fn shutdown(&self, deadline: Instant) -> Vec<WorkerShutdownDiagnostic> {
        self.faults.clear_handler();
        let (root, subscription) = {
            let mut state = self.state();
            state.rows.iter_mut().for_each(HostedRow::cancel_restart);
            (
                std::mem::replace(&mut state.root, HostRoot::Stopped),
                state.subscription.take(),
            )
        };
        drop(subscription);
        let HostRoot::Running(root) = root else {
            return Vec::new();
        };
        let disposal = root.fiber().dispose();
        match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), disposal).await {
            Ok(Ok(())) => Vec::new(),
            Ok(Err(error)) => vec![WorkerShutdownDiagnostic {
                worker: K::SHUTDOWN_WORKER.to_string(),
                message: PluginError::from(error).to_string(),
            }],
            Err(_) => vec![WorkerShutdownDiagnostic {
                worker: K::SHUTDOWN_WORKER.to_string(),
                message: format!("{}s did not stop before the shutdown deadline", K::NOUN),
            }],
        }
    }

    /// Stops the rows one by one in reverse mount order, each bounded by its
    /// stop bound plus `grace` (a row without one by `grace`) and by
    /// `deadline`, then the root. A row not stopped yet still serves, so a
    /// row's stop can reach the rows mounted before it. A row that does not
    /// stop in time is reported and left stopping while the next one
    /// stops. Nothing is mounted afterwards. For a host whose rows depend
    /// on the ones mounted before them; [`Self::shutdown`] stops them all
    /// at once.
    pub async fn shutdown_in_reverse(
        &self,
        deadline: Instant,
        grace: Duration,
    ) -> Vec<WorkerShutdownDiagnostic> {
        self.faults.clear_handler();
        let (root, subscription, fibers) = {
            let mut state = self.state();
            state.rows.iter_mut().for_each(HostedRow::cancel_restart);
            let fibers = state
                .rows
                .iter()
                .rev()
                .filter_map(|row| match &row.mount {
                    Mount::Mounted(fiber) if !fiber.is_removed() => Some((
                        row.name.clone(),
                        fiber.clone(),
                        row.stop_bound.unwrap_or_default() + grace,
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let root = match &state.root {
                HostRoot::Running(root) => {
                    let root = root.clone();
                    state.root = HostRoot::Stopping(root.clone());
                    Some(root)
                }
                HostRoot::NotStarted | HostRoot::Stopping(_) | HostRoot::Stopped => {
                    state.root = HostRoot::Stopped;
                    None
                }
            };
            (root, state.subscription.take(), fibers)
        };
        drop(subscription);
        let Some(root) = root else {
            return Vec::new();
        };
        let mut diagnostics = Vec::new();
        for (name, fiber, row_bound) in fibers {
            let bound = deadline.min(Instant::now() + row_bound);
            let disposal = fiber.dispose();
            match tokio::time::timeout_at(tokio::time::Instant::from_std(bound), disposal).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => diagnostics.push(WorkerShutdownDiagnostic {
                    worker: K::SHUTDOWN_WORKER.to_string(),
                    message: format!(
                        "{} {name} did not stop cleanly: {}",
                        K::NOUN,
                        PluginError::from(error)
                    ),
                }),
                Err(_) => diagnostics.push(WorkerShutdownDiagnostic {
                    worker: K::SHUTDOWN_WORKER.to_string(),
                    message: format!("{} {name} did not stop in time", K::NOUN),
                }),
            }
        }
        self.state().root = HostRoot::Stopped;
        let disposal = root.fiber().dispose();
        if let Ok(Err(error)) =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), disposal).await
        {
            diagnostics.push(WorkerShutdownDiagnostic {
                worker: K::SHUTDOWN_WORKER.to_string(),
                message: PluginError::from(error).to_string(),
            });
        }
        diagnostics
    }

    /// Logs how a configuration update of row `name` ended.
    pub async fn report_reconfigured(name: String, settle: Settle) {
        match settle.await {
            Ok(()) => log::info!("{} {name} applied its new configuration", K::NOUN),
            Err(error) => log::error!(
                "{} {name} failed with its new configuration: {}",
                K::NOUN,
                PluginError::from(error)
            ),
        }
    }

    /// Spawns one of the host's own tasks on the ambient runtime; a panic in
    /// it is logged and reported to the profile, and the task ends with
    /// `None`.
    fn spawn_task<F>(&self, future: F) -> JoinHandle<Option<F::Output>>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let profile = self.profile.clone();
        tokio::spawn(async move {
            match catch_panic_async(future).await {
                Ok(output) => Some(output),
                Err(message) => {
                    log::error!("the {} host's task panicked: {message}", K::NOUN);
                    profile.host_task_panicked(&message);
                    None
                }
            }
        })
    }

    fn state(&self) -> MutexGuard<'_, HostState<K>> {
        self.state.lock()
    }
}
