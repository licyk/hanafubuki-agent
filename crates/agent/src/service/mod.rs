use super::host::AgentHostOptions;
use super::host::{
    agent_plugin_manifest, AgentHost, AgentPluginEventSink, AgentPluginInputs, AgentPluginRow,
};
use super::skills::AgentSkillApplication;
use super::titles::AgentTitleApplication;
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

mod background_terminals;
mod history;
mod mcp;
pub(crate) use mcp::AgentMcpApplication;
pub use mcp::{
    McpApplicationSnapshot, McpCredentialPresence, McpCredentialsWriteRequest,
    McpLocalPolicySummary, McpLocalServerStatus, McpLocalStatusSnapshot, McpOAuthCallbackRequest,
    McpOAuthCancelRequest, McpOAuthLoginRequest, McpServerDeleteRequest, McpServerWriteRequest,
    McpToolSnapshot, SkillMcpDependencyStatus,
};
mod plugins;
pub use plugins::AgentSettingsView;
mod projections;
pub use projections::AgentRunProjection;
mod provider;
pub(crate) use provider::{
    resolve_provider_definition, resolve_settings_provider, ResolvedAgentProvider,
    ResolvedModelPolicy,
};
pub use provider::{
    AgentModelPolicyWriteRequest, AgentProviderDeleteRequest, AgentProviderDraft,
    AgentProviderEnabledRequest, AgentProviderModelCatalogRequest, AgentProviderModelCatalogResult,
    AgentProviderRegistrySnapshot, AgentProviderWriteRequest, AgentSessionModelSelectionRequest,
    AgentSessionReasoningEffortRequest,
};
mod sessions;
mod skill_operations;
pub use skill_operations::{AgentSkillSessionState, LocalSkillInstallRequest};
#[cfg(test)]
mod test_access;
mod turns;

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
    /// What the embedding host supplies.
    pub host: AgentHostPorts,
    /// This instance's start options, fault sink and Executor.
    pub instance: AgentInstanceInputs,
}

/// The host interface: what an embedding application adds to the Agent.
/// Built for every Agent instance, since plugin rows are mounted once.
pub struct AgentHostPorts {
    /// The host's plugins, mounted after the Agent's capability plugins and
    /// before its context plugins.
    pub plugins: Vec<AgentPluginRow>,
    /// Where the Agent works.
    pub workspace: Arc<dyn WorkspaceRootsPort>,
    /// The context steering carries.
    pub steering_context: Arc<dyn HostContextPort>,
    /// What the host adds to the base instructions.
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
    /// The workspace policy, for user image attachments and their preview.
    pub(crate) files: WorkspaceFsService,
    /// Where this instance's kernel faults go (the slot that started it).
    pub(crate) kernel_faults: Arc<dyn hanafubuki_agent_kernel::AgentKernelFaultSink>,
}

impl AgentApplicationService {
    /// Establish a coherent raw Agent storage boundary for App diagnostics.
    pub fn flush_for_diagnostics(&self) -> Result<(), String> {
        self.core
            .flush_for_diagnostics()
            .map_err(|error| error.to_string())
    }

    /// The test Agent: built and recovered.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new_with_host(
        config: AgentConfig,
        host: AgentHostPorts,
        storage_paths: AgentStoragePaths,
        skill_roots: SkillRoots,
        executor: ExecutorManager,
    ) -> Result<Self, String> {
        let service = Self::construct(
            AgentServiceInputs {
                config,
                storage_paths,
                skill_roots,
                blocking: Arc::new(crate::test_support::TestBlockingOperationPort),
                mcp_event_sink: Arc::new(hanafubuki_agent_mcp::NoopMcpEventSink),
                plugin_event_sink: Arc::new(super::host::NoopAgentPluginEventSink),
                host,
                instance: AgentInstanceInputs {
                    executor: executor.owned_by(
                        hanafubuki_agent_process::process_groups::ProcessOwnerGroup::new(),
                    ),
                    ..AgentInstanceInputs::default()
                },
            },
            hanafubuki_agent_host::RestartPolicy::default(),
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
                host,
                instance,
            },
            hanafubuki_agent_host::RestartPolicy::default(),
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
        extra: Vec<AgentPluginRow>,
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
                host,
                instance: AgentInstanceInputs::default(),
            },
            hanafubuki_agent_host::RestartPolicy {
                delays: Vec::new(),
                window: std::time::Duration::ZERO,
            },
        )?;
        service.recover()?;
        Ok(service)
    }

    /// Builds the Agent from `inputs`. Its durable sessions are not loaded
    /// yet: the caller runs [`Self::recover`] next, before anything else uses
    /// the service.
    pub fn open(inputs: AgentServiceInputs) -> Result<Self, String> {
        Self::construct(inputs, hanafubuki_agent_host::RestartPolicy::default())
    }

    fn construct(
        inputs: AgentServiceInputs,
        plugin_restarts: hanafubuki_agent_host::RestartPolicy,
    ) -> Result<Self, String> {
        let AgentServiceInputs {
            config,
            storage_paths,
            skill_roots,
            blocking,
            mcp_event_sink,
            plugin_event_sink,
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
        let files = WorkspaceFsService::new(
            WorkspaceFsPolicy::new(workspace_roots.into_vec(), workspace.extra_read_roots())
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
        let mcp_application = AgentMcpApplication::open(
            &host.network.proxy()?,
            &executor,
            workers.clone(),
            mcp_event_sink,
            config.mcp().clone(),
            Arc::new(FoundationMcpSecretStore::new(host.secrets)),
            blocking.clone(),
        )?;
        let mcp = mcp_application.mcp.clone();
        let history_paths = AgentHistoryPaths::under(storage_paths.history_root.clone());
        let history_store = Arc::new(
            if instance.options.rebuild_history_index {
                FileAgentHistoryStore::open_rebuilding_index(
                    history_paths,
                    workers.clone(),
                    instance.history_refusals.clone(),
                )
            } else {
                FileAgentHistoryStore::open_reporting_refusals(
                    history_paths,
                    workers.clone(),
                    instance.history_refusals.clone(),
                )
            }
            .map_err(|error| error.to_string())?,
        );
        let memory_repository = AgentMemoryRepository::open(
            hanafubuki_agent_memory::AgentMemoryPaths::under(storage_paths.memory_root),
        )
        .map_err(|error| error.to_string())?;
        // The kernel reads the registries the host's plugins fill; the
        // `titles` plugin reads and titles the kernel's sessions.
        let registries = AgentKernelRegistries::default();
        let core = AgentService::without_default_model_reporting_faults(
            registries.clone(),
            history_store.clone(),
            approvals.clone(),
            kernel_faults.clone(),
        );
        let memory = AgentMemoryApplication::new(
            config.clone(),
            core.clone(),
            memory_repository.clone(),
            history_store,
            memory_workers.clone(),
        )?;
        let plugin_host = Arc::new(AgentHost::new(
            registries,
            blocking.clone(),
            config.clone(),
            host.network.clone(),
            plugin_event_sink,
            agent_plugin_manifest(
                AgentPluginInputs {
                    user_instructions_dir: storage_paths.root.clone(),
                    workspace: workspace.clone(),
                    files: files.clone(),
                    executions: executions.clone(),
                    mcp: mcp.clone(),
                    memory: memory.clone(),
                    skills: skills.clone(),
                    mcp_application: mcp_application.clone(),
                    titles: AgentTitleApplication::new(core.clone(), config.clone()),
                },
                host.plugins,
            ),
            AgentHostOptions {
                safe_mode: instance.options.safe_mode,
                kernel_faults: kernel_faults.clone(),
                restart_policy: plugin_restarts,
                ..AgentHostOptions::default()
            },
        )?);
        unbuilt.built();
        Ok(Self {
            config,
            core,
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
            base_instructions: agent_base_instructions(&host.instructions).into(),
            host: plugin_host,
            files,
            kernel_faults,
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
