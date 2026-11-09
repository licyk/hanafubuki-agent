//! An Agent built over a temporary directory laid out as an application's
//! data directory, with a host's ports.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_config::{AgentMemorySettings, AgentProviderSettings, AgentSettingsPatch};
use hanafubuki_agent_contracts::TurnExtensions;
use hanafubuki_agent_foundation::blocking::{
    BlockingFuture, BlockingOperation, BlockingOperationPort,
};
use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_foundation::secrets::MemorySecretStore;
use hanafubuki_agent_process::executor::ExecutorManager;

/// A host that adds no context to steering.
pub use crate::NoHostContext;
use crate::{
    agent_workspace_roots_from, AgentApplicationService, AgentConfig, AgentHostInstructions,
    AgentHostPorts, AgentMemoryApplication, AgentModelProtocol, AgentProxy, AgentStoragePaths,
    AgentWorkspaceRoots, NetworkSettingsPort, SkillRoots, WorkspaceRootsPort,
};

/// What a test gives the Agent's shutdown before it expects no diagnostics.
/// A session's history writer flushes on stop, which under a loaded machine
/// can take longer than the stop itself takes in practice.
pub const TEST_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);

/// How long a test waits for something it expects to happen. Only the
/// outcome matters, so the wait is generous: a loaded CI runner gets there
/// seconds late.
pub const TEST_WAIT: Duration = Duration::from_secs(60);

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

/// A temporary directory laid out as an application's data directory.
#[derive(Clone, Debug)]
pub struct TestPaths {
    /// The directory itself, which a test removes when it ends.
    pub app_data_dir: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    /// The Agent's configuration directory.
    pub config_dir: PathBuf,
    /// The workspace the test host gives the Agent.
    pub managed_instances_dir: PathBuf,
}

/// A fresh [`TestPaths`] under the temporary directory, its directories
/// created.
pub fn test_paths(name: &str) -> TestPaths {
    let id = NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "hanafubuki-agent-application-{name}-{}-{id}",
        std::process::id()
    ));
    let paths = TestPaths {
        data_dir: root.join("data"),
        logs_dir: root.join("logs"),
        config_dir: root.join("config").join("agent"),
        managed_instances_dir: root.join("instances"),
        app_data_dir: root,
    };
    for directory in [
        &paths.data_dir,
        &paths.logs_dir,
        &paths.managed_instances_dir,
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    paths
}

/// A host with no plugins, steering context or instructions, whose
/// workspace is the managed instance root, which may read the application's
/// data and logs, and whose proxy is the system's.
pub fn test_host(paths: &TestPaths) -> AgentHostPorts {
    let root = ResolvedPath::resolve(&paths.managed_instances_dir).unwrap();
    AgentHostPorts {
        plugins: Vec::new(),
        workspace: Arc::new(FixedWorkspaceRoots {
            roots: agent_workspace_roots_from([root.native().to_path_buf()]).unwrap(),
            protected_roots: vec![paths.app_data_dir.clone(), paths.logs_dir.clone()],
        }),
        steering_context: Arc::new(NoHostContext),
        instructions: AgentHostInstructions::default(),
        network: Arc::new(FixedProxy(AgentProxy::System)),
        secrets: Arc::new(MemorySecretStore::default()),
    }
}

/// A host whose proxy never changes.
pub struct FixedProxy(pub AgentProxy);

impl NetworkSettingsPort for FixedProxy {
    fn proxy(&self) -> Result<AgentProxy, String> {
        Ok(self.0.clone())
    }
}

/// Roots that never change; nothing a host submits selects or adds one.
pub struct FixedWorkspaceRoots {
    pub roots: AgentWorkspaceRoots,
    pub protected_roots: Vec<PathBuf>,
}

impl WorkspaceRootsPort for FixedWorkspaceRoots {
    fn roots(&self) -> Result<AgentWorkspaceRoots, String> {
        Ok(self.roots.clone())
    }

    fn protected_roots(&self) -> Vec<PathBuf> {
        self.protected_roots.clone()
    }

    fn selected(&self, _host: &TurnExtensions) -> Option<PathBuf> {
        None
    }

    fn search_roots(&self, _host: &TurnExtensions) -> Result<Vec<(String, PathBuf)>, String> {
        Ok(Vec::new())
    }
}

/// Builds and recovers an Agent over [`test_paths`] after `configure` ran
/// against its paths and configuration, so a test decides exactly which
/// provider and credential state exists (including none), with the ports
/// `host` builds.
pub fn service_with_host(
    name: &str,
    host: impl FnOnce(&TestPaths) -> AgentHostPorts,
    configure: impl FnOnce(&TestPaths, &AgentConfig),
) -> (AgentApplicationService, TestPaths, AgentConfig) {
    let paths = test_paths(name);
    let config = AgentConfig::new(&paths.config_dir);
    configure(&paths, &config);
    let host = host(&paths);
    let service = AgentApplicationService::new_with_host(
        config.clone(),
        host,
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap();
    (service, paths, config)
}

/// One enabled Responses provider at `base_url`, the default, with the model
/// `fixture-model`; and `sandboxMode = "full-access"`, since a test binary
/// is no sandbox helper (`sandbox::helper_dispatch`), so a sandboxed
/// command would fail there. A test of the sandbox sets another mode.
pub fn configure_test_provider(
    config: &AgentConfig,
    base_url: &str,
    reasoning_effort: Option<hanafubuki_agent_kernel::ModelReasoningEffort>,
) {
    config
        .write_settings(crate::AgentSettingsPatch {
            sandbox_mode: Some(Some(crate::AgentSandboxMode::FullAccess)),
            ..crate::AgentSettingsPatch::default()
        })
        .unwrap();
    config
        .mutate_providers(0, |registry| {
            registry.default_provider_id = Some("provider-1".into());
            registry.model_policy.model = "fixture-model".into();
            registry.model_policy.reasoning_effort = reasoning_effort;
            registry.providers.push(AgentProviderSettings {
                id: "provider-1".into(),
                display_name: "Fixture Provider".into(),
                enabled: true,
                protocol: AgentModelProtocol::Responses,
                base_url: base_url.into(),
                api_token: None,
                request_headers: Vec::new(),
            });
            Ok(())
        })
        .unwrap();
}

/// Writes two enabled System Skills, so a turn's request lists a catalog.
pub fn enable_two_skills(service: &AgentApplicationService, paths: &TestPaths) {
    for (folder, description) in [
        ("diagnostics", "Explain terminal diagnostics and task logs"),
        ("release-notes", "Write release notes from a changelog"),
    ] {
        let package = paths
            .data_dir
            .join("agent-test/skills-bundled")
            .join(folder);
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            format!("---\nname: {folder}\ndescription: {description}\n---\nFollow the {folder} workflow.\n"),
        )
        .unwrap();
    }
    for entry in service.list_skills().unwrap().entries {
        service.enable_skill(&entry.id).unwrap();
    }
}

/// Settings with memory on: retrieval, and generation when `generate`.
/// Without generation no memory pass follows a turn, so none competes with
/// the test for the fake model.
pub fn enable_memory(config: &AgentConfig, generate: bool) {
    config
        .write_settings(AgentSettingsPatch {
            memory: Some(AgentMemorySettings {
                enabled: true,
                generate_memories: generate,
                ..AgentMemorySettings::default()
            }),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
}

/// Runs blocking work on the Tokio blocking pool, as the platform port does,
/// so a tool parked inside a domain call never stalls a test's runtime. Work
/// started outside a runtime runs inline.
pub struct TestBlockingOperationPort;

impl BlockingOperationPort for TestBlockingOperationPort {
    fn execute<'a>(
        &'a self,
        label: &'static str,
        operation: BlockingOperation,
    ) -> BlockingFuture<'a> {
        Box::pin(async move {
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => runtime
                    .spawn_blocking(operation)
                    .await
                    .map_err(|_| format!("{label} failed")),
                Err(_) => Ok(operation()),
            }
        })
    }
}

impl AgentApplicationService {
    pub fn test_memory(&self) -> AgentMemoryApplication {
        self.memory().clone()
    }
}
