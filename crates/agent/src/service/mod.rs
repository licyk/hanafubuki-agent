use super::host::AgentHostOptions;
use super::host::{
    agent_plugin_manifest, AgentHost, AgentPluginDescriptor, AgentPluginEventSink,
    AgentPluginInputs, AgentPluginRow,
};
use super::model_discovery::{ModelDiscovery, ModelDiscoveryPolicy};
use super::model_metadata_store::FileModelMetadataStore;
use super::session_driver::AgentSessionDriver;
use super::skills::AgentSkillApplication;
use super::titles::AgentTitleApplication;
use super::wake::{AgentBackgroundEventSink, Wakes};
use super::{
    agent_base_instructions, AgentHostInstructions, AgentInstanceInputs, AgentMemoryApplication,
    AgentStoragePaths, HostContextPort, NetworkSettingsPort, SkillRoots, WorkspaceRootsPort,
};
use hanafubuki_agent_approval::ApprovalService;
use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_execution::AgentExecutionService;
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::secrets::SecretStore;
use hanafubuki_agent_history::{AgentHistoryPaths, FileAgentHistoryStore};
use hanafubuki_agent_kernel::{AgentKernelRegistries, AgentService, AgentSessionId};
use hanafubuki_agent_mcp::{FoundationMcpSecretStore, McpEventSink, McpManager};
use hanafubuki_agent_memory::AgentMemoryRepository;
#[cfg(any(test, feature = "test-support"))]
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_workspace_fs::{WorkspaceFsPolicy, WorkspaceFsService};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

mod background;
pub use background::AgentBackgroundStatus;
mod background_terminals;
mod children;
mod compression;
mod export;
mod features;
mod goal;
mod history;
mod mcp;
pub(crate) use mcp::AgentMcpApplication;
pub use mcp::{
    McpApplicationSnapshot, McpCredentialPresence, McpCredentialsWriteRequest,
    McpLocalPolicySummary, McpLocalServerStatus, McpLocalStatusSnapshot, McpOAuthCallbackRequest,
    McpOAuthCancelRequest, McpOAuthLoginRequest, McpPluginServer, McpPluginServerSnapshot,
    McpServerDeleteRequest, McpServerWriteRequest, McpToolSnapshot, SkillMcpDependencyStatus,
};
mod plugins;
pub use plugins::AgentSettingsView;
mod projections;
mod review;
pub use projections::AgentRunProjection;
mod provider;
pub(crate) use provider::{
    configured_model_facts, resolve_provider_definition, resolve_settings_provider,
    ResolvedAgentProvider, ResolvedModelPolicy,
};
pub use provider::{
    AgentListedModel, AgentModelPolicyWriteRequest, AgentProviderDeleteRequest, AgentProviderDraft,
    AgentProviderEnabledRequest, AgentProviderModelCatalogRequest, AgentProviderModelCatalogResult,
    AgentProviderRegistrySnapshot, AgentProviderWriteRequest, AgentSessionModelSelectionRequest,
    AgentSessionReasoningEffortRequest,
};
mod sessions;
pub use sessions::AgentEditFromTurn;
mod settings;
pub use settings::{
    package_setting_key, reasoning_effort_choices, SettingsAction, SettingsActionOutcome,
    SettingsActionRequest, SettingsActionScope, SettingsCatalog, SettingsCollection,
    SettingsRecord,
};
mod skill_operations;
pub use skill_operations::{AgentSkillSessionState, LocalSkillInstallRequest};
#[cfg(test)]
mod test_access;
mod tui_preferences;
pub use tui_preferences::AgentStandaloneSettingsView;
mod turns;
mod workspace;
mod worktree;
pub use worktree::AgentWorktree;
mod hooks;
pub use hooks::{AgentHook, AgentHookHandler, AgentHooks};
mod guests;
mod packages;
mod roles;

/// What the application hands the Agent when it builds it.
pub struct AgentServiceInputs {
    /// The Agent's configuration, in the directory the host names.
    pub config: AgentConfig,
    /// The Agent's storage, under the data directory the host names.
    pub storage_paths: AgentStoragePaths,
    pub skill_roots: SkillRoots,
    /// The Agent runtime's blocking pool (`ContainedRuntime::blocking_port`).
    pub blocking: Arc<dyn hanafubuki_agent_foundation::blocking::BlockingOperationPort>,
    pub mcp_event_sink: Arc<dyn McpEventSink>,
    pub plugin_event_sink: Arc<dyn AgentPluginEventSink>,
    /// Told when a root is to be woken for its background work.
    pub background_event_sink: Arc<dyn AgentBackgroundEventSink>,
    /// What the embedding host supplies.
    pub host: AgentHostPorts,
    /// This instance's start options, fault sink and Executor.
    pub instance: AgentInstanceInputs,
}

/// The host interface: what an embedding application adds to the Agent.
/// Built for every Agent instance, since plugin rows are mounted once.
pub struct AgentHostPorts {
    /// The host's plugins, mounted after the Agent's capability plugins and
    /// before its context plugins, each with its descriptor: its name (the
    /// row's plugin name), its summary and its options.
    pub plugins: Vec<(AgentPluginRow, AgentPluginDescriptor)>,
    /// Where the Agent works.
    pub workspace: Arc<dyn WorkspaceRootsPort>,
    /// The context steering carries.
    pub steering_context: Arc<dyn HostContextPort>,
    /// Who the Agent is, and what the host adds to the base instructions.
    pub instructions: AgentHostInstructions,
    /// The proxy the Agent's HTTP clients use.
    pub network: Arc<dyn NetworkSettingsPort>,
    /// Where MCP OAuth tokens are kept (the platform's secure store).
    pub secrets: Arc<dyn SecretStore>,
}

#[derive(Clone)]
pub struct AgentApplicationService {
    config: AgentConfig,
    pub(crate) core: AgentService,
    /// Drives child sessions, and prepares every turn the facade admits.
    driver: AgentSessionDriver,
    /// Held while a side conversation is found or opened, so a parent never
    /// gets two.
    side_conversations: Arc<parking_lot::Mutex<()>>,
    executions: AgentExecutionService,
    skills: AgentSkillApplication,
    memory: AgentMemoryApplication,
    accepting: Arc<AtomicBool>,
    workers: BackgroundWorkerGroup,
    memory_workers: BackgroundWorkerGroup,
    mcp: Arc<McpManager>,
    mcp_application: AgentMcpApplication,
    blocking: Arc<dyn hanafubuki_agent_foundation::blocking::BlockingOperationPort>,
    provider_configuration_gate: Arc<tokio::sync::Mutex<()>>,
    /// The host's network settings, which the MCP client follows.
    network: Arc<dyn NetworkSettingsPort>,
    /// The host's workspace roots, which file search also reads.
    pub(crate) workspace: Arc<dyn WorkspaceRootsPort>,
    /// Renders the host context steering carries.
    pub(crate) steering_context: Arc<dyn HostContextPort>,
    /// The base instructions with the host's additions.
    pub(crate) base_instructions: Arc<str>,
    host: Arc<AgentHost>,
    /// The installed plugins, whose content the Skills, MCP and hooks follow.
    packages: crate::packages::AgentPackages,
    /// Told of the plugins' changes, and of the installed plugins'.
    plugin_events: Arc<dyn AgentPluginEventSink>,
    /// The roles, rebuilt whenever the settings or the packages are.
    roles: Arc<crate::roles::AgentRoles>,
    /// The workspace policy, for user image attachments and their preview.
    pub(crate) files: WorkspaceFsService,
    /// Where this instance's kernel faults go (the slot that started it).
    pub(crate) kernel_faults: Arc<dyn hanafubuki_agent_kernel::AgentKernelFaultSink>,
    /// Lists the Providers' models in the background.
    discovery: ModelDiscovery,
    /// The host's background sink and the roots' pauses, shared with the
    /// `agent-wake` core row.
    pub(crate) wakes: Wakes,
}

impl AgentApplicationService {
    /// Establish a coherent raw Agent storage boundary for App diagnostics.
    pub fn flush_for_diagnostics(&self) -> Result<(), String> {
        self.core
            .flush_for_diagnostics()
            .map_err(|error| error.to_string())
    }

    /// The test Agent: built and recovered. It discovers no models by
    /// itself, whatever its settings say, and tells no one of its wakes.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new_with_host(
        config: AgentConfig,
        host: AgentHostPorts,
        storage_paths: AgentStoragePaths,
        skill_roots: SkillRoots,
        executor: ExecutorManager,
    ) -> Result<Self, String> {
        Self::new_with_host_discovering(
            config,
            host,
            storage_paths,
            skill_roots,
            executor,
            ModelDiscoveryPolicy::OFF,
            Arc::new(super::wake::NoopAgentBackgroundEventSink),
        )
    }

    /// [`Self::new_with_host`], discovering models under `discovery` and
    /// telling `background` of its wakes.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new_with_host_discovering(
        config: AgentConfig,
        host: AgentHostPorts,
        storage_paths: AgentStoragePaths,
        skill_roots: SkillRoots,
        executor: ExecutorManager,
        discovery: ModelDiscoveryPolicy,
        background: Arc<dyn AgentBackgroundEventSink>,
    ) -> Result<Self, String> {
        let service = Self::construct(
            AgentServiceInputs {
                config,
                storage_paths,
                skill_roots,
                blocking: Arc::new(crate::test_support::TestBlockingOperationPort),
                mcp_event_sink: Arc::new(hanafubuki_agent_mcp::NoopMcpEventSink),
                plugin_event_sink: Arc::new(super::host::NoopAgentPluginEventSink),
                background_event_sink: background,
                host,
                instance: AgentInstanceInputs {
                    executor: executor.owned_by(
                        hanafubuki_agent_process::process_groups::ProcessOwnerGroup::new(),
                    ),
                    ..AgentInstanceInputs::default()
                },
            },
            hanafubuki_agent_host::RestartPolicy::default(),
            discovery,
        )?;
        service.recover()?;
        Ok(service)
    }

    /// The test Agent over [`crate::test_support::test_host`]: built and
    /// recovered.
    #[cfg(test)]
    pub(crate) fn new(
        paths: &crate::test_support::TestPaths,
        config: AgentConfig,
        storage_paths: AgentStoragePaths,
        skill_roots: SkillRoots,
        executor: ExecutorManager,
    ) -> Result<Self, String> {
        let host = crate::test_support::test_host(paths);
        Self::new_with_host(config, host, storage_paths, skill_roots, executor)
    }

    /// The test Agent before recovery, as composition's constructor returns
    /// it to the start-up sequence.
    #[cfg(test)]
    pub(crate) fn new_unrecovered(
        config: AgentConfig,
        host: AgentHostPorts,
        storage_paths: AgentStoragePaths,
        skill_roots: SkillRoots,
        instance: AgentInstanceInputs,
    ) -> Result<Self, String> {
        Self::construct(
            AgentServiceInputs {
                config,
                storage_paths,
                skill_roots,
                blocking: Arc::new(crate::test_support::TestBlockingOperationPort),
                mcp_event_sink: Arc::new(hanafubuki_agent_mcp::NoopMcpEventSink),
                plugin_event_sink: Arc::new(super::host::NoopAgentPluginEventSink),
                background_event_sink: Arc::new(super::wake::NoopAgentBackgroundEventSink),
                host,
                instance,
            },
            hanafubuki_agent_host::RestartPolicy::default(),
            ModelDiscoveryPolicy::OFF,
        )
    }

    /// The test Agent with `extra` as the host's plugin rows: test plugins
    /// that run beside the real ones. A faulted plugin is not restarted by
    /// itself, so a test can watch it stay failed until its retry;
    /// `tests::plugin_host` covers the automatic restart.
    #[cfg(test)]
    pub(crate) fn new_with_test_plugins(
        paths: &crate::test_support::TestPaths,
        config: AgentConfig,
        storage_paths: AgentStoragePaths,
        skill_roots: SkillRoots,
        extra: Vec<(AgentPluginRow, AgentPluginDescriptor)>,
    ) -> Result<Self, String> {
        let mut host = crate::test_support::test_host(paths);
        host.plugins = extra;
        let service = Self::construct(
            AgentServiceInputs {
                config,
                storage_paths,
                skill_roots,
                blocking: Arc::new(crate::test_support::TestBlockingOperationPort),
                mcp_event_sink: Arc::new(hanafubuki_agent_mcp::NoopMcpEventSink),
                plugin_event_sink: Arc::new(super::host::NoopAgentPluginEventSink),
                background_event_sink: Arc::new(super::wake::NoopAgentBackgroundEventSink),
                host,
                instance: AgentInstanceInputs::default(),
            },
            hanafubuki_agent_host::RestartPolicy {
                delays: Vec::new(),
                window: std::time::Duration::ZERO,
            },
            ModelDiscoveryPolicy::OFF,
        )?;
        service.recover()?;
        Ok(service)
    }

    /// Builds the Agent from `inputs`. Its durable sessions are not loaded
    /// yet: the caller runs [`Self::recover`] next, before anything else uses
    /// the service.
    pub fn open(inputs: AgentServiceInputs) -> Result<Self, String> {
        Self::construct(
            inputs,
            hanafubuki_agent_host::RestartPolicy::default(),
            ModelDiscoveryPolicy::AUTOMATIC,
        )
    }

    fn construct(
        inputs: AgentServiceInputs,
        plugin_restarts: hanafubuki_agent_host::RestartPolicy,
        discovery: ModelDiscoveryPolicy,
    ) -> Result<Self, String> {
        let AgentServiceInputs {
            config,
            storage_paths,
            skill_roots,
            blocking,
            mcp_event_sink,
            plugin_event_sink,
            background_event_sink,
            host,
            instance,
        } = inputs;
        // Every process this instance starts is in its owner group.
        let executor = instance.executor.clone();
        let kernel_faults = instance.kernel_faults.clone();
        let accepting = Arc::new(AtomicBool::new(true));
        // Loaded before the plugin host derives its rows from them.
        config.settings()?;
        let executions = AgentExecutionService::new(executor.clone());
        let workspace = host.workspace;
        let workspace_roots = workspace.roots()?;
        log::info!(
            "agent workspace roots resolved count={} primary={}",
            workspace_roots.roots().len(),
            workspace_roots.primary().display()
        );
        executions.set_workspace_roots(workspace_roots.roots().to_vec());
        let files = WorkspaceFsService::new(
            WorkspaceFsPolicy::new(workspace_roots.into_vec(), workspace.protected_roots())
                .map_err(|error| error.to_string())?,
        );
        // A catalog that cannot open leaves the Agent without Skills, which
        // the `skills` plugin reports as its failure.
        let skills = AgentSkillApplication::open(skill_roots, Some(workspace.clone()));
        let approvals = ApprovalService::manual();
        // The Agent's threads: history writers, MCP stdio readers and the
        // memory supervisor. Contained, like the Agent runtime's; a member
        // that panics is a kernel fault.
        let worker_faults = kernel_faults.clone();
        let on_panic: hanafubuki_agent_foundation::background_worker::WorkerPanicHandler =
            Arc::new(move |worker: &str, message: &str| {
                worker_faults.kernel_fault(&format!("worker {worker}"), message);
            });
        let workers = BackgroundWorkerGroup::contained_with_panic_handler(on_panic.clone());
        let memory_workers = BackgroundWorkerGroup::contained_with_panic_handler(on_panic);
        // A step below that fails stops what the steps before it started.
        let unbuilt = StopWorkersUnlessBuilt::new([memory_workers.clone(), workers.clone()]);
        let proxy = host.network.proxy()?;
        let mcp_application = AgentMcpApplication::open(
            &proxy,
            &executor,
            workers.clone(),
            mcp_event_sink,
            config.mcp().clone(),
            Arc::new(FoundationMcpSecretStore::new(host.secrets)),
            blocking.clone(),
        )?;
        let mcp = mcp_application.mcp.clone();
        let (packages, hooks) = packages::open_packages(
            &storage_paths,
            instance.options.safe_mode,
            &executor,
            &config,
            &skills,
            &mcp_application,
            |guests| {
                crate::packages::guest_host(
                    &executor,
                    &config,
                    blocking.clone(),
                    workspace.clone(),
                    workers.clone(),
                    guests,
                )
            },
        )?;
        let history_store = open_history_store(&storage_paths, &instance, &workers)?;
        let memory_repository = AgentMemoryRepository::open(
            hanafubuki_agent_memory::AgentMemoryPaths::under(storage_paths.memory_root),
        )
        .map_err(|error| error.to_string())?;
        // The kernel reads the registries the host's plugins fill; the
        // `titles` plugin reads and titles the kernel's sessions.
        let registries = AgentKernelRegistries::default();
        let model_metadata = Arc::new(FileModelMetadataStore::open(
            storage_paths.model_metadata_file.clone(),
            &workers,
        )?);
        let core = AgentService::without_default_model_reporting_faults(
            registries.clone(),
            history_store.clone(),
            approvals.clone(),
            kernel_faults.clone(),
            model_metadata,
        );
        core.set_model_proxy(crate::network::model_http_proxy(proxy));
        let discovery = ModelDiscovery::start(discovery, core.clone(), config.clone(), &workers)?;
        let memory = AgentMemoryApplication::new(
            config.clone(),
            core.clone(),
            memory_repository.clone(),
            history_store,
            memory_workers.clone(),
        )?;
        let base_instructions: Arc<str> = agent_base_instructions(&host.instructions).into();
        let wakes = Wakes::new(background_event_sink);
        let driver = AgentSessionDriver::new(
            core.clone(),
            config.clone(),
            registries.clone(),
            executions.clone(),
            blocking.clone(),
            base_instructions.clone(),
            discovery.clone(),
        );
        let roles = Arc::new(crate::roles::AgentRoles::load(
            &config,
            packages.loaded().roles(),
        ));
        let plugin_host = Arc::new(AgentHost::new(
            registries,
            blocking.clone(),
            config.clone(),
            host.network.clone(),
            plugin_event_sink.clone(),
            agent_plugin_manifest(
                AgentPluginInputs {
                    user_instructions_dir: storage_paths.root.clone(),
                    application: host.instructions.application.clone(),
                    workspace: workspace.clone(),
                    files: files.clone(),
                    executions: executions.clone(),
                    mcp: mcp.clone(),
                    memory: memory.clone(),
                    skills: skills.clone(),
                    mcp_application: mcp_application.clone(),
                    titles: AgentTitleApplication::new(core.clone(), config.clone()),
                    conversations: crate::conversation_reader::AgentConversationReader::new(
                        core.clone(),
                        config.clone(),
                    ),
                    goals: crate::goals::AgentGoalApplication::new(core.clone()),
                    driver: driver.clone(),
                    roles: crate::roles::AgentRoleCatalogReader::new(roles.clone()),
                    worktrees: hanafubuki_agent_worktree::WorktreeManager::new(
                        storage_paths.worktrees_root.clone(),
                        executor.clone(),
                    ),
                    git: hanafubuki_agent_worktree::GitQueries::new(executor.clone()),
                    hooks,
                    packages: crate::packages::PackageReload {
                        packages: packages.clone(),
                        config: config.clone(),
                        blocking: blocking.clone(),
                        roles: roles.clone(),
                        faults: kernel_faults.clone(),
                    },
                    working_directory: hooks::working_directory(workspace.clone()),
                    wakes: wakes.clone(),
                },
                host.plugins,
            ),
            AgentHostOptions {
                safe_mode: instance.options.safe_mode,
                kernel_faults: kernel_faults.clone(),
                restart_policy: plugin_restarts,
            },
        )?);
        unbuilt.built();
        Ok(Self {
            config,
            core,
            driver,
            side_conversations: Arc::new(parking_lot::Mutex::new(())),
            executions,
            skills,
            memory,
            accepting,
            workers,
            memory_workers,
            mcp,
            mcp_application,
            blocking,
            provider_configuration_gate: Arc::new(tokio::sync::Mutex::new(())),
            network: host.network,
            workspace,
            steering_context: host.steering_context,
            base_instructions,
            host: plugin_host,
            packages,
            plugin_events: plugin_event_sink,
            roles,
            files,
            kernel_faults,
            discovery,
            wakes,
        })
    }

    /// Loads the durable sessions, once, before the service is used. Each
    /// session is materialized with an unavailable transport: the exact
    /// Provider is resolved per session before its next model operation, so
    /// one global Provider can never retarget history. A session that fails
    /// to load is quarantined and listed in the report; only a failure to
    /// read the history index fails recovery.
    pub fn recover(&self) -> Result<hanafubuki_agent_kernel::AgentRecoveryReport, String> {
        let report = self
            .core
            .recover_sessions()
            .map_err(|error| error.to_string())?;
        if report.restored > 0 {
            log::info!(
                "restored {} durable Agent conversation sessions",
                report.restored
            );
        }
        if !report.quarantined.is_empty() {
            log::error!(
                "{} Agent conversation session(s) quarantined; their files are kept",
                report.quarantined.len()
            );
        }
        self.reconcile_recovered_execution_activities();
        Ok(report)
    }

    pub fn memory(&self) -> &AgentMemoryApplication {
        &self.memory
    }

    /// Read an `apply_patch` argument into per-file diffs for the interface.
    ///
    /// The interface carried its own reader until this existed, and the two
    /// disagreed: a patch this backend rejected could still be painted as a
    /// clean diff, so the user saw `+1 −1` on a file the tool had refused.
    /// Rust owns the format; the interface renders what Rust read.
    ///
    /// Pure and bounded — no filesystem, no policy, no resolution. Whether the
    /// paths exist, lie in the workspace, or still match their preimage is
    /// decided by `prepare`, not here.
    pub fn preview_patch(&self, patch: &str) -> hanafubuki_agent_workspace_fs::PatchPreview {
        hanafubuki_agent_workspace_fs::preview_patch(patch)
    }

    /// The workspace roots as the host's port reads them now, the one
    /// relative paths resolve against first.
    pub fn workspace_roots(&self) -> Result<crate::AgentWorkspaceRoots, String> {
        self.workspace.roots()
    }

    pub fn memory_status(&self) -> Result<hanafubuki_agent_memory::AgentMemoryStatus, String> {
        self.memory.status()
    }

    pub fn memory_session_contribution(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(hanafubuki_agent_memory::AgentMemoryContributionMode, u64), String> {
        self.activate_session(session_id)
            .map_err(|error| error.to_string())?;
        let state = self
            .core
            .session_memory_contribution(session_id)
            .map_err(|error| error.to_string())?;
        Ok((state.mode, state.revision))
    }

    pub fn set_memory_session_contribution(
        &self,
        session_id: AgentSessionId,
        expected_revision: u64,
        enabled: bool,
    ) -> Result<(hanafubuki_agent_memory::AgentMemoryContributionMode, u64), String> {
        self.activate_session(session_id)
            .map_err(|error| error.to_string())?;
        // The memory plugin's observer records the committed state in the
        // memory store, and signals a memory scan, before this returns.
        let state = self
            .core
            .set_session_memory_contribution(session_id, expected_revision, enabled)
            .map_err(|error| error.to_string())?;
        Ok((state.mode, state.revision))
    }

    pub fn reset_memory(&self, expected_generation: u64) -> Result<u64, String> {
        self.memory.reset(expected_generation)
    }

    /// What the reads of the configuration files had to fix since the
    /// configuration was opened, oldest first.
    pub fn config_recoveries(&self) -> Vec<hanafubuki_agent_config::AgentConfigRecovery> {
        self.config.recoveries()
    }

    /// What the layers of `config.toml` hold that the settings cannot use:
    /// unknown keys, values the layer below replaces, and plugin options
    /// that do not fit their plugin's.
    pub fn config_layer_warnings(&self) -> Result<Vec<String>, String> {
        let mut warnings = self.config.layer_warnings()?;
        warnings.extend(self.host.option_issues(&self.config.settings()?));
        Ok(warnings)
    }
}

/// The history store under `storage_paths`, its index rebuilt first when
/// `instance`'s start asks for it.
fn open_history_store(
    storage_paths: &AgentStoragePaths,
    instance: &AgentInstanceInputs,
    workers: &BackgroundWorkerGroup,
) -> Result<Arc<FileAgentHistoryStore>, String> {
    let paths = AgentHistoryPaths::under(storage_paths.history_root.clone());
    let refusals = instance.history_refusals.clone();
    if instance.options.rebuild_history_index {
        FileAgentHistoryStore::open_rebuilding_index(paths, workers.clone(), refusals)
    } else {
        FileAgentHistoryStore::open_reporting_refusals(paths, workers.clone(), refusals)
    }
    .map(Arc::new)
    .map_err(|error| error.to_string())
}

/// Stops the Agent's worker groups when its construction fails part-way:
/// the history store and the memory supervisor run threads in them before
/// the last steps can still fail, and a service that was never built has no
/// shutdown to stop them.
struct StopWorkersUnlessBuilt {
    groups: Option<[BackgroundWorkerGroup; 2]>,
}

impl StopWorkersUnlessBuilt {
    fn new(groups: [BackgroundWorkerGroup; 2]) -> Self {
        Self {
            groups: Some(groups),
        }
    }

    fn built(mut self) {
        self.groups = None;
    }
}

impl Drop for StopWorkersUnlessBuilt {
    fn drop(&mut self) {
        let Some(groups) = self.groups.take() else {
            return;
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        for group in groups {
            for diagnostic in group.shutdown(deadline) {
                log::warn!(
                    "a failed Agent construction left worker {} running: {}",
                    diagnostic.worker,
                    diagnostic.message
                );
            }
        }
    }
}
