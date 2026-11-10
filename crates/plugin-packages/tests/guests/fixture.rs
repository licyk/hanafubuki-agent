//! A guest host mounted in a kardia root of its own, over an executor in an
//! owner group, with packages whose guest is this binary.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_contracts::{
    ContextRegistry, ContextRendererRegistry, ContinuationHookRegistry, ObserverRegistry,
    PluginFaults, PromptContributorRegistry, RegistryService, ToolLifecycleHookRegistry,
    ToolSourceRegistry,
};
use hanafubuki_agent_host::{LogExporter, RestartPolicy};
use hanafubuki_agent_packages::{
    content_hash, PackageContributes, PackagePermissions, PackageRuntime, PluginId, RuntimeKind,
    WorkspaceAccess,
};
use hanafubuki_agent_plugin_packages::{
    GuestHost, GuestHostInputs, GuestPackage, GuestSettingsPort, GuestStatus,
};
use hanafubuki_agent_process::executor::manager::executor_registry_counts;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_process::process_groups::{ProcessGroupRecord, ProcessOwnerGroup};
use hanafubuki_agent_sandbox::Sandbox;
use kardia_core::{Context, Fiber, PluginDef, Service, Svc};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::guest::GUEST_ARG;

/// How long a check waits for an outcome before it fails.
pub const DEADLINE: Duration = Duration::from_secs(60);

pub struct Options {
    /// `None`: the guests run unconfined.
    pub sandbox: Option<Sandbox>,
    pub call_timeout: Duration,
    pub policy: RestartPolicy,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            sandbox: None,
            call_timeout: Duration::from_secs(20),
            policy: RestartPolicy {
                delays: vec![Duration::ZERO; 3],
                window: Duration::from_secs(600),
            },
        }
    }
}

/// What the loader tree wrote to the settings.
#[derive(Default)]
pub struct RecordedSettings(pub Mutex<Vec<(String, bool)>>);

impl GuestSettingsPort for RecordedSettings {
    fn set_enabled(&self, id: &str, enabled: bool) {
        self.0.lock().push((id.to_string(), enabled));
    }
}

pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub host: GuestHost,
    pub executor: ExecutorManager,
    pub group: ProcessOwnerGroup,
    /// The owner group's `process-groups.json`.
    pub record: ProcessGroupRecord,
    pub settings: Arc<RecordedSettings>,
    pub plugin: Fiber,
    pub workspace: PathBuf,
    root: Context,
}

impl Fixture {
    pub async fn start(options: Options) -> Self {
        let directory = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let workspace = directory.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let record = ProcessGroupRecord::new(directory.path().join("process-groups.json"));
        let group = ProcessOwnerGroup::recorded_in(record.clone());
        let executor = ExecutorManager::default().owned_by(group.clone());
        let settings = Arc::new(RecordedSettings::default());
        let roots = vec![workspace.clone()];
        let inputs = GuestHostInputs {
            executor: executor.clone(),
            sandbox: options
                .sandbox
                .clone()
                .unwrap_or_else(|| Sandbox::unavailable("unconfined")),
            settings: settings.clone(),
            workspace: Arc::new(move || roots.clone()),
            restart_policy: options.policy,
            call_timeout: options.call_timeout,
        };
        let host = match options.sandbox {
            Some(_) => GuestHost::new(inputs, Vec::new()),
            None => GuestHost::unconfined(inputs, Vec::new()),
        };
        let root = Context::new();
        root.add_exporter(Arc::new(LogExporter)).unwrap();
        // What the kernel provides: the faults and the registries guests
        // contribute to.
        let services = PluginDef::new(|ctx: Context, _| async move {
            ctx.provide(PluginFaults::default())?;
            ctx.provide(ToolSourceRegistry::default())?;
            ctx.provide(ContextRegistry::default())?;
            ctx.provide(ContextRendererRegistry::default())?;
            ctx.provide(PromptContributorRegistry::default())?;
            ctx.provide(ToolLifecycleHookRegistry::default())?;
            ctx.provide(ContinuationHookRegistry::default())?;
            ctx.provide(ObserverRegistry::default())?;
            Ok(())
        })
        .provides([
            PluginFaults::NAME,
            ToolSourceRegistry::NAME,
            ContextRegistry::NAME,
            ContextRendererRegistry::NAME,
            PromptContributorRegistry::NAME,
            ToolLifecycleHookRegistry::NAME,
            ContinuationHookRegistry::NAME,
            ObserverRegistry::NAME,
        ]);
        root.plugin_def(&services, Value::Null)
            .unwrap()
            .settled()
            .await
            .unwrap();
        let plugin = root.plugin_def(&host.plugin(), Value::Null).unwrap();
        plugin.settled().await.unwrap();
        Self {
            directory,
            host,
            executor,
            group,
            record,
            settings,
            plugin,
            workspace,
            root,
        }
    }

    /// The registry `S` the guests contribute to.
    pub fn registry<S: RegistryService>(&self) -> Svc<S> {
        self.root.get::<S>().unwrap()
    }

    /// A package named `name`, on, with its permissions granted, whose
    /// guest is this binary in `mode`, started as an interpreter outside
    /// the package.
    pub fn package(&self, name: &str, mode: &str) -> GuestPackage {
        let root = self.directory.path().join("packages").join(name);
        fs::create_dir_all(root.join(".hanafubuki-plugin")).unwrap();
        fs::write(
            root.join(".hanafubuki-plugin/plugin.json"),
            json!({ "name": name, "runtime": { "type": "process" } }).to_string(),
        )
        .unwrap();
        fs::write(root.join("package.txt"), "inside the package").unwrap();
        let program = std::env::current_exe().unwrap();
        let permissions = PackagePermissions {
            network: false,
            workspace: WorkspaceAccess::None,
            env: vec!["HANAFUBUKI_TEST_GRANTED".to_string()],
        };
        GuestPackage {
            id: PluginId::new(name, "local").unwrap(),
            version: "local".to_string(),
            content_hash: Some(content_hash(&root).unwrap()),
            data_root: self.directory.path().join("data").join(name),
            root,
            name: name.to_string(),
            runtime: PackageRuntime {
                kind: RuntimeKind::Process,
                command: vec![
                    program.to_str().unwrap().to_string(),
                    GUEST_ARG.to_string(),
                    name.to_string(),
                    mode.to_string(),
                ],
                interpreter: true,
                protocol: Some("kardia-guest/2".to_string()),
            },
            granted: Some(permissions.clone()),
            permissions,
            contributes: PackageContributes::default(),
            settings: Map::new(),
            enabled: true,
        }
    }

    /// The status of `id` once `until` holds.
    pub async fn wait(
        &self,
        id: &str,
        what: &str,
        until: impl Fn(&GuestStatus) -> bool,
    ) -> GuestStatus {
        let mut changes = self.host.changes();
        let deadline = Instant::now() + DEADLINE;
        loop {
            let status = self.host.status(id);
            if let Some(status) = status.as_ref().filter(|status| until(status)) {
                return status.clone();
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "{id}: no {what} within {DEADLINE:?}: {status:#?}"
            );
            let _ = tokio::time::timeout(remaining, changes.changed()).await;
        }
    }

    /// The guest processes the executor still holds.
    pub fn processes(&self) -> usize {
        executor_registry_counts(&self.executor).0
    }

    /// Waits until the executor holds `count` processes.
    pub async fn wait_for_processes(&self, count: usize) {
        eventually(&format!("{count} guest processes"), || {
            self.processes() == count
        })
        .await;
    }

    pub async fn call(
        &self,
        package: &GuestPackage,
        method: &str,
        args: Value,
    ) -> Result<Value, String> {
        self.host
            .call(&package.id, method, args, Duration::from_secs(20))
            .await
    }
}

/// Waits until `condition` holds, checking it again every few
/// milliseconds, for at most [`DEADLINE`].
pub async fn eventually(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !condition() {
        assert!(Instant::now() < deadline, "no {what} within {DEADLINE:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Running, with a live process (a process that just ended leaves its
/// guest running until its fiber fails).
pub fn running(status: &GuestStatus) -> bool {
    status.state == hanafubuki_agent_plugin_packages::GuestState::Running
        && status.process_id.is_some()
}

/// Kills the process `pid` as `kill -9` does.
pub fn kill_hard(pid: u32) {
    let status = if cfg!(windows) {
        std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .status()
    } else {
        std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
    };
    assert!(status.unwrap().success(), "the guest {pid} was not killed");
}
