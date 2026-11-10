//! The guest host's handle, which the facade keeps: the packages whose code
//! runs, each guest's status, retry, and a call into a guest.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_contracts::PluginFaults;
use hanafubuki_agent_foundation::panic::register_containment_check;
use hanafubuki_agent_host::{RestartPolicy, GUEST_LOGGER_PREFIX};
use hanafubuki_agent_packages::{PluginId, RuntimeKind};
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::Sandbox;
use kardia_core::{Fiber, FiberState};
use kardia_hmr::{GuestFailure, GuestFailureKind, GuestService};
use kardia_loader::Loader;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::package::{GuestPackage, GUEST_SERVICE};
use crate::resolver::GuestResolver;
use crate::settings::GuestSettingsPort;
use crate::spawn::{SpawnContext, WorkspaceRoots};
use crate::state::{GuestRecord, GuestState, GuestStatus};
use crate::supervise::Restart;

/// What the guest host is built from.
pub struct GuestHostInputs {
    /// The instance's executor, in its owner group: every guest process is
    /// its.
    pub executor: ExecutorManager,
    /// The platform's sandbox, which confines every guest by its plugin
    /// profile; a guest does not start where it is unavailable.
    pub sandbox: Sandbox,
    /// Where the loader tree's own changes are written.
    pub settings: Arc<dyn GuestSettingsPort>,
    /// The workspace a guest may read or write, as granted.
    pub workspace: WorkspaceRoots,
    /// When a guest that failed while running starts again.
    pub restart_policy: RestartPolicy,
    /// The bound on a guest's handshake and on each export without a
    /// deadline of its own; past it, the guest's process is killed.
    pub call_timeout: Duration,
}

/// The code of the installed plugins: one out-of-process Kardia guest per
/// enabled code package, in the `plugins` row's kardia-loader subtree
/// ([`GuestHost::plugin`]). Clones share the guests.
#[derive(Clone)]
pub struct GuestHost(pub(crate) Arc<Shared>);

pub(crate) struct Shared {
    pub(crate) context: Arc<SpawnContext>,
    pub(crate) policy: RestartPolicy,
    pub(crate) settings: Arc<dyn GuestSettingsPort>,
    pub(crate) resolver: GuestResolver,
    pub(crate) packages: Mutex<Vec<GuestPackage>>,
    mount: Mutex<Option<Mount>>,
    pub(crate) records: Mutex<BTreeMap<String, GuestRecord>>,
    /// Held by an application of the packages to the tree, so the later of
    /// two is the one that stays.
    applying: tokio::sync::Mutex<()>,
    changes: watch::Sender<u64>,
}

/// The guest host while its plugin runs.
#[derive(Clone)]
pub(crate) struct Mount {
    pub(crate) loader: Loader,
    pub(crate) restarts: mpsc::UnboundedSender<Restart>,
    /// Where the kernel records what it went without from a guest.
    pub(crate) faults: Arc<PluginFaults>,
}

impl GuestHost {
    /// The guest host for `packages`, which start once its plugin runs.
    pub fn new(inputs: GuestHostInputs, packages: Vec<GuestPackage>) -> Self {
        let sandbox = Some(inputs.sandbox.clone());
        Self::build(inputs, sandbox, packages)
    }

    /// [`Self::new`] whose guests run without the sandbox, for tests on a
    /// machine that has none.
    #[cfg(any(test, feature = "test-support"))]
    pub fn unconfined(inputs: GuestHostInputs, packages: Vec<GuestPackage>) -> Self {
        Self::build(inputs, None, packages)
    }

    fn build(
        inputs: GuestHostInputs,
        sandbox: Option<Sandbox>,
        packages: Vec<GuestPackage>,
    ) -> Self {
        // kardia-hmr's guest threads catch their panics: the Agent's panic
        // hook must let them unwind (before any of them starts).
        register_containment_check(kardia_hmr::is_panic_contained);
        let context = Arc::new(SpawnContext {
            executor: inputs.executor,
            sandbox,
            workspace: inputs.workspace,
            call_timeout: inputs.call_timeout,
        });
        let shared = Arc::new(Shared {
            context,
            policy: inputs.restart_policy,
            settings: inputs.settings,
            resolver: GuestResolver::default(),
            packages: Mutex::new(Vec::new()),
            mount: Mutex::new(None),
            records: Mutex::new(BTreeMap::new()),
            applying: tokio::sync::Mutex::new(()),
            changes: watch::Sender::new(0),
        });
        shared.follow(packages);
        Self(shared)
    }

    /// Applies `packages`, the installed packages with code, on or off:
    /// a guest starts for each that is on and may start, and stops for one
    /// that is off or gone. Returns once the guests are started or stopped,
    /// not once they run.
    pub async fn apply(&self, packages: Vec<GuestPackage>) {
        self.0.follow(packages);
        self.0.reconcile().await;
    }

    /// The status of the guest of package `id`, if it has code.
    pub fn status(&self, id: &str) -> Option<GuestStatus> {
        let mut status = {
            let records = self.0.records.lock();
            records
                .get(id)
                .map(|record| record.status(&self.0.policy))?
        };
        if let Some(mount) = self.0.mount() {
            status
                .issues
                .extend(mount.faults.issues(&format!("{GUEST_LOGGER_PREFIX}{id}")));
        }
        Some(status)
    }

    /// A counter that changes with every guest's status.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.0.changes.subscribe()
    }

    /// Starts the guest of `id` again after it failed, with an empty restart
    /// window. Returns once it is started, not once it runs.
    pub async fn retry(&self, id: &PluginId) -> Result<(), String> {
        let id = id.key();
        let fiber = self
            .0
            .current_fiber(&id)
            .filter(|fiber| fiber.state() == FiberState::Failed)
            .ok_or_else(|| format!("the guest of plugin `{id}` has not failed"))?;
        {
            let mut records = self.0.records.lock();
            let record = records
                .get_mut(&id)
                .ok_or_else(|| format!("plugin `{id}` has no code"))?;
            record.window.clear();
            record.epoch += 1;
            record.state = GuestState::Starting;
            record.error = None;
        }
        self.0.changed();
        log::info!("plugin {id}: its guest is retried");
        restart_fiber(&fiber).await;
        Ok(())
    }

    /// Calls `method` of the service the guest of `id` provides
    /// ([`GUEST_SERVICE`]), with `args`, killing the guest if it has not
    /// answered within `timeout`. Dropping the future cancels the call. A
    /// guest whose instance failed in the call ([`GuestFailure`]: a WASM
    /// trap, its fuel, time or memory used up, a panic serving it) is
    /// failed, as a process guest that exits is, and restarts under the
    /// restart policy; an error the guest reports itself is the call's.
    pub async fn call(
        &self,
        id: &PluginId,
        method: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = id.key();
        let fiber = self
            .0
            .current_fiber(&id)
            .filter(|fiber| fiber.state() == FiberState::Active)
            .ok_or_else(|| format!("the guest of plugin `{id}` is not running"))?;
        let service = fiber
            .context()
            .live_as::<GuestService>(GUEST_SERVICE)
            .ok_or_else(|| format!("the guest of plugin `{id}` provides no service"))?;
        let error = match service.call_async(method, args, Some(timeout)).await {
            Ok(answer) => return Ok(answer),
            Err(error) => error,
        };
        let runtime = self.0.package(&id).map(|package| package.runtime.kind());
        if let Some(failure) =
            GuestFailure::of(&error).filter(|failure| fails_guest(failure, runtime))
        {
            log::warn!(
                "plugin {id}: its guest's instance failed in a call ({:?})",
                failure.kind
            );
            drop(fiber.fail(failure.clone().into()));
        }
        Err(error.to_string())
    }
}

/// Whether a call's failure is the guest's to answer by failing its fiber.
/// A process guest that ended or was killed (also at its deadline) is not:
/// kardia-hmr's exit watcher fails its fiber with the exit report.
fn fails_guest(failure: &GuestFailure, runtime: Option<RuntimeKind>) -> bool {
    match failure.kind {
        GuestFailureKind::Exited | GuestFailureKind::Killed => false,
        GuestFailureKind::Timeout => runtime == Some(RuntimeKind::Wasm),
        _ => true,
    }
}

impl Shared {
    /// Takes `packages` as the set from now on, with each guest's state
    /// where it can be told without the loader: a guest that may run is
    /// `starting` while the guest host runs, `stopped` otherwise.
    fn follow(&self, packages: Vec<GuestPackage>) {
        let mounted = self.mount().is_some();
        let mut previous = self.packages.lock();
        let mut records = self.records.lock();
        records.retain(|id, _| packages.iter().any(|package| package.id.key() == *id));
        for package in &packages {
            let before = previous.iter().find(|known| known.id == package.id);
            let record = records
                .entry(package.id.key())
                .or_insert_with(|| GuestRecord::new(GuestState::Stopped));
            if before == Some(package) {
                continue;
            }
            record.epoch += 1;
            record.started = false;
            if before.is_none_or(|before| !before.runs()) {
                record.window.clear();
            }
            (record.state, record.error) = match (package.enabled, package.refusal()) {
                (false, _) => (GuestState::Stopped, None),
                (true, Some(refusal)) => (GuestState::Blocked, Some(refusal)),
                (true, None) if mounted => (GuestState::Starting, None),
                (true, None) => (GuestState::Stopped, None),
            };
        }
        drop(records);
        *previous = packages;
        drop(previous);
        self.changed();
    }

    /// Brings the loader's entries in step with the packages: its group
    /// update starts, rebuilds and stops the guests, and commits nothing to
    /// the settings, which the entries come from. One at a time, each with
    /// the packages as they are when it starts, so the last one stays.
    pub(crate) async fn reconcile(&self) {
        let _applying = self.applying.lock().await;
        let Some(mount) = self.mount() else {
            return;
        };
        let packages = self.packages.lock().clone();
        self.resolver.follow(&packages, &self.context);
        let entries = packages.iter().flat_map(GuestPackage::entries).collect();
        mount.loader.root().update(entries).await;
    }

    pub(crate) fn mount(&self) -> Option<Mount> {
        self.mount.lock().clone()
    }

    pub(crate) fn mounted(&self, mount: Mount) {
        *self.mount.lock() = Some(mount);
    }

    /// The guest host's plugin stopped, and its guests with it.
    pub(crate) fn unmounted(&self) {
        *self.mount.lock() = None;
        let mut records = self.records.lock();
        for record in records.values_mut() {
            if record.state != GuestState::Blocked {
                record.state = GuestState::Stopped;
                record.handle = None;
                record.epoch += 1;
            }
        }
        drop(records);
        self.changed();
    }

    /// The fiber of `id`'s loader entry.
    pub(crate) fn current_fiber(&self, id: &str) -> Option<Fiber> {
        self.mount()?.loader.resolve(id).ok()?.inner_fiber()
    }

    pub(crate) fn changed(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }
}

/// Clears a failed guest's error and starts it again, without writing its
/// configuration back.
pub(crate) async fn restart_fiber(fiber: &Fiber) {
    match fiber.update_with(fiber.config(), true) {
        Ok(settle) => {
            let _ = settle.await;
        }
        Err(error) => log::warn!("a plugin guest was not restarted: {error}"),
    }
}
