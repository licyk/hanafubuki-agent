//! Start-up after the application is presented: the slot ends `Running` or
//! `Failed`, a failure never reaches the caller, and the crash-loop marker
//! counts only starts that did not finish. Then what bounds a start and the
//! Agent's shutdown: MCP connects off the start's path, a start that
//! shutdown cancels, a shutdown stage that hangs, and saved settings that
//! never wait for a reconnect.

use std::sync::atomic::AtomicBool;

use crate::skill_watcher::SkillCatalogWatcher;
use crate::test_support::TestPaths;
use crate::AgentConfig;

use super::*;
use crate::test_support::test_paths;

const SETTLE: Duration = Duration::from_millis(50);

#[derive(Default)]
pub(super) struct CountingStatusSink(pub(super) AtomicUsize);

impl AgentStatusEventSink for CountingStatusSink {
    fn status_changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn skill_roots(paths: &TestPaths) -> SkillRoots {
    SkillRoots {
        system: paths.data_dir.join("agent-test/skills-bundled"),
        user: paths.data_dir.join("agent-test/skills-user"),
        install_bundled_system_skills: false,
        state_file: paths.data_dir.join("agent-test/skills-state.json"),
    }
}

pub(super) fn unrecovered(paths: &TestPaths, storage: AgentStoragePaths) -> AgentConstructor {
    unrecovered_with(
        paths,
        storage,
        AgentConfig::new(&paths.config_dir),
        Vec::new(),
    )
}

/// A constructor with the settings state and extra plugin rows a test
/// watches.
pub(super) fn unrecovered_with(
    paths: &TestPaths,
    storage: AgentStoragePaths,
    config: AgentConfig,
    extra: Vec<(
        super::host::AgentPluginRow,
        super::host::AgentPluginDescriptor,
    )>,
) -> AgentConstructor {
    let paths = paths.clone();
    // The extra rows go to the first instance; later ones run without them.
    let extra = Mutex::new(Some(extra));
    Arc::new(move |_runtime, instance| {
        let mut host = test_host(&paths);
        host.plugins = extra.lock().unwrap().take().unwrap_or_default();
        AgentApplicationService::new_unrecovered(
            config.clone(),
            host,
            storage.clone(),
            skill_roots(&paths),
            instance,
        )
    })
}

/// The launch a test starts the Agent with: the given Agent Executor, an
/// idle Skill watcher, and a process-group ledger under the test's Agent
/// root.
pub(super) fn launcher(
    paths: &TestPaths,
    marker: &AgentStartMarker,
    construct: AgentConstructor,
    executor: hanafubuki_agent_process::executor::ExecutorManager,
) -> AgentLauncher {
    AgentLauncher::new(
        AgentLaunch {
            construct,
            process_groups_file: storage_of(paths).process_groups_file,
        },
        executor,
        SkillCatalogWatcher::idle(),
        marker.clone(),
        SETTLE,
    )
}

pub(super) fn marker(paths: &TestPaths) -> AgentStartMarker {
    AgentStartMarker::new(
        AgentStoragePaths::under(paths.data_dir.join("agent-test")).start_marker_file,
    )
}

/// Runs the start from a thread outside any runtime, as composition's
/// start does: the start builds the Agent instance's runtime itself.
pub(super) fn start(
    slot: &AgentSlot,
    marker: &AgentStartMarker,
    construct: AgentConstructor,
) -> Option<AgentHandle> {
    start_with(
        slot,
        AgentLauncher::new(
            AgentLaunch {
                construct,
                // The test's own ledger: a start reaps what the ledger lists.
                process_groups_file: marker.file_for_test().with_file_name("process-groups.json"),
            },
            hanafubuki_agent_process::executor::ExecutorManager::default(),
            SkillCatalogWatcher::idle(),
            marker.clone(),
            SETTLE,
        ),
    )
}

/// Runs the start of `launcher` from a thread outside any runtime.
pub(super) fn start_with(slot: &AgentSlot, launcher: AgentLauncher) -> Option<AgentHandle> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(start_agent(slot.clone(), launcher))
}

pub(super) fn stop(agent: AgentHandle) {
    assert_eq!(agent.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE), []);
}

/// Stops the Agent instance's runtime, as shutdown does last.
pub(super) fn stop_runtime(slot: &AgentSlot) {
    if let Some(runtime) = slot.take_runtime() {
        runtime.shutdown(Duration::from_secs(2)).unwrap();
    }
}

#[test]
fn a_failing_constructor_marks_the_agent_failed_and_its_callers_unavailable() {
    let paths = test_paths("startup-construct-fails");
    let events = Arc::new(CountingStatusSink::default());
    let slot = AgentSlot::new(None, events.clone());
    let marker = marker(&paths);

    let agent = start(
        &slot,
        &marker,
        Arc::new(|_, _| Err("injected construction failure".to_string())),
    );

    assert!(agent.is_none());
    let status = slot.status();
    assert_eq!(
        status.availability,
        AgentAvailability::Failed {
            stage: AgentStartStage::Construct,
            reason: "injected construction failure".to_string(),
        }
    );
    assert_eq!(
        slot.agent().err().unwrap().to_string(),
        "the Agent is unavailable: its construction failed: injected construction failure"
    );
    assert!(events.0.load(Ordering::SeqCst) > 0);
    // The failure was orderly, so it is not an unfinished start.
    assert_eq!(marker.unfinished_starts(), 0);
    // Task Center reads the slot: no Agent, no Agent rows.
    assert!(slot.list_run_projections().is_empty());
    assert!(slot.get_run_projection("agent-run-1").is_err());
    // A second start is not attempted.
    assert!(start(
        &slot,
        &marker,
        unrecovered(
            &paths,
            AgentStoragePaths::under(paths.data_dir.join("agent-test"))
        )
    )
    .is_none());
    stop_runtime(&slot);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn an_unopenable_history_store_fails_construction_without_reaching_the_caller() {
    let paths = test_paths("startup-history-unopenable");
    let agent_root = paths.data_dir.join("agent-test");
    std::fs::create_dir_all(&agent_root).unwrap();
    // The history directory is a file, so the store cannot open.
    std::fs::write(agent_root.join("history"), b"not a directory").unwrap();
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));

    let agent = start(
        &slot,
        &marker(&paths),
        unrecovered(&paths, AgentStoragePaths::under(agent_root)),
    );

    assert!(agent.is_none());
    assert!(matches!(
        slot.status().availability,
        AgentAvailability::Failed {
            stage: AgentStartStage::Construct,
            ..
        }
    ));
    stop_runtime(&slot);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn an_agent_disabled_by_setting_is_never_constructed() {
    let paths = test_paths("startup-disabled");
    let marker = marker(&paths);
    let slot = AgentSlot::new(
        marker.disabled_reason(false),
        Arc::new(NoopAgentStatusEventSink),
    );
    let constructed = Arc::new(AtomicBool::new(false));
    let probe = constructed.clone();

    let agent = start(
        &slot,
        &marker,
        Arc::new(move |_, _| {
            probe.store(true, Ordering::SeqCst);
            Err("constructed".to_string())
        }),
    );

    assert!(agent.is_none());
    assert!(!constructed.load(Ordering::SeqCst));
    assert_eq!(
        slot.status().availability,
        AgentAvailability::Disabled {
            reason: AgentDisabledReason::Setting
        }
    );
    assert_eq!(
        slot.agent().err().unwrap().to_string(),
        "the Agent is unavailable: it is turned off in Settings"
    );
    // No attempt was recorded for an Agent that never started.
    assert!(!marker.file_exists_for_test());
    stop_runtime(&slot);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn three_unfinished_starts_turn_the_agent_off_until_the_marker_is_reset() {
    let paths = test_paths("startup-crash-loop");
    for _ in 0..CRASH_LOOP_STARTS {
        // A process that died during start-up: an attempt and nothing else.
        assert_eq!(marker(&paths).disabled_reason(true), None);
        marker(&paths).record_attempt().unwrap();
    }
    let marker = marker(&paths);
    assert_eq!(marker.unfinished_starts(), CRASH_LOOP_STARTS);
    assert_eq!(
        marker.disabled_reason(true),
        Some(AgentDisabledReason::CrashLoop)
    );
    // An orderly exit of a crash-looped run records nothing and clears
    // nothing.
    marker.finish();
    assert_eq!(
        marker.disabled_reason(true),
        Some(AgentDisabledReason::CrashLoop)
    );
    // The setting off wins over the crash loop.
    assert_eq!(
        marker.disabled_reason(false),
        Some(AgentDisabledReason::Setting)
    );
    marker.reset().unwrap();
    assert_eq!(marker.unfinished_starts(), 0);
    assert_eq!(marker.disabled_reason(true), None);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn a_running_agent_clears_its_start_attempt_once_it_settles() {
    let paths = test_paths("startup-settles");
    let marker = marker(&paths);
    marker.record_attempt().unwrap();
    let slot = AgentSlot::new(
        marker.disabled_reason(true),
        Arc::new(NoopAgentStatusEventSink),
    );

    let agent = start(
        &slot,
        &marker,
        unrecovered(
            &paths,
            AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        ),
    )
    .expect("the Agent starts");

    assert_eq!(agent.generation(), 1);
    assert_eq!(slot.status().availability, AgentAvailability::Running);
    assert_eq!(slot.status().generation, 1);
    assert_eq!(slot.agent().unwrap().generation(), 1);
    // This start is recorded on top of the earlier unfinished one.
    assert_eq!(marker.unfinished_starts(), 2);
    let deadline = Instant::now() + TEST_WAIT;
    while marker.unfinished_starts() != 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(marker.unfinished_starts(), 0);

    match slot.close(Instant::now()) {
        AgentClosing::Running(closing) => assert_eq!(closing.generation(), 1),
        AgentClosing::Abandoned => panic!("the running Agent was abandoned"),
        AgentClosing::NotRunning(state) => {
            panic!("the running Agent was not handed over: {state:?}")
        }
    }
    assert_eq!(slot.status().availability, AgentAvailability::Stopped);
    assert!(slot.agent().is_err());
    stop(agent);
    stop_runtime(&slot);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn a_broken_session_is_quarantined_and_listed_while_the_others_load() {
    let (service, paths) = configured_service("startup-quarantine", "http://127.0.0.1:1/v1");
    let setup = tokio::runtime::Runtime::new().unwrap();
    let [broken, intact] = [0, 1].map(|_| {
        setup
            .block_on(service.create_session(AgentSessionMode::Default, None))
            .unwrap()
            .id
    });
    drop(setup);
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    drop(service);
    let storage = AgentStoragePaths::under(paths.data_dir.join("agent-test"));
    let index = hanafubuki_agent_history::AgentHistoryPaths::under(storage.history_root.clone())
        .sqlite_database;
    {
        let connection = rusqlite::Connection::open(&index).unwrap();
        connection
            .execute("UPDATE agent_sessions SET recovery_needed = 1", [])
            .unwrap();
        connection
            .execute(
                "UPDATE agent_sessions SET runtime_json = '{broken' WHERE session_id = ?",
                [i64::try_from(broken.value()).unwrap()],
            )
            .unwrap();
    }

    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let agent = start(&slot, &marker(&paths), unrecovered(&paths, storage))
        .unwrap_or_else(|| panic!("a broken session stopped the Agent: {:?}", slot.status()));

    let status = slot.status();
    assert_eq!(status.availability, AgentAvailability::Running);
    assert_eq!(
        status
            .quarantined_sessions
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        vec![broken]
    );
    assert!(agent.open_session(intact, 75).is_ok());
    assert!(agent.open_session(broken, 75).is_err());
    let kept: String = rusqlite::Connection::open(&index)
        .unwrap()
        .query_row(
            "SELECT runtime_json FROM agent_sessions WHERE session_id = ?",
            [i64::try_from(broken.value()).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kept, "{broken");
    stop(agent);
    stop_runtime(&slot);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A history item that does not decode, in a session with nothing to
/// recover, is refused by the history store as it opens; the store still
/// opens, the Agent runs, and the session is in the status's quarantine.
#[test]
fn an_unreadable_history_item_quarantines_its_session_and_the_agent_runs() {
    let (service, paths) = configured_service("startup-unreadable-item", "http://127.0.0.1:1/v1");
    let setup = tokio::runtime::Runtime::new().unwrap();
    let [broken, intact] = [0, 1].map(|_| {
        setup
            .block_on(service.create_session(AgentSessionMode::Default, None))
            .unwrap()
            .id
    });
    drop(setup);
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    drop(service);
    let storage = AgentStoragePaths::under(paths.data_dir.join("agent-test"));
    let index = hanafubuki_agent_history::AgentHistoryPaths::under(storage.history_root.clone())
        .sqlite_database;
    let broken_id = i64::try_from(broken.value()).unwrap();
    {
        let connection = rusqlite::Connection::open(&index).unwrap();
        let recovering: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_sessions WHERE recovery_needed = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recovering, 0);
        // A new session has no item yet: the index gains one it cannot read.
        connection
            .execute(
                "INSERT INTO agent_history_items(session_id, turn_id, item_id, first_record_ordinal, first_item_order, item_revision, created_at, item_type, item_json, session_revision) VALUES (?, NULL, 1, 0, 0, 0, '0', 'user_message', '{broken', 0)",
                [broken_id],
            )
            .unwrap();
    }

    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let agent = start(&slot, &marker(&paths), unrecovered(&paths, storage))
        .unwrap_or_else(|| panic!("an unreadable item stopped the Agent: {:?}", slot.status()));

    let status = slot.status();
    assert_eq!(status.availability, AgentAvailability::Running);
    assert_eq!(
        status
            .quarantined_sessions
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        vec![broken]
    );
    assert!(agent.open_session(intact, 75).is_ok());
    assert!(agent.open_session(broken, 75).is_err());
    stop(agent);
    stop_runtime(&slot);
    let kept: String = rusqlite::Connection::open(&index)
        .unwrap()
        .query_row(
            "SELECT item_json FROM agent_history_items WHERE session_id = ? LIMIT 1",
            [broken_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kept, "{broken");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn a_built_agent_that_shutdown_overtakes_is_retired_not_installed() {
    let paths = test_paths("startup-overtaken");
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let built = Arc::new(Mutex::new(None));
    let construct = {
        let built = built.clone();
        let closing = slot.clone();
        let inner = unrecovered(
            &paths,
            AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        );
        Arc::new(
            move |runtime: &ContainedRuntime, instance: AgentInstanceInputs| {
                let service = inner(runtime, instance)?;
                *built.lock().unwrap() = Some(service.clone());
                // Shutdown gives up waiting while the Agent is still starting.
                assert!(matches!(
                    closing.close(Instant::now()),
                    AgentClosing::Abandoned
                ));
                Ok(service)
            },
        ) as AgentConstructor
    };

    assert!(start(&slot, &marker(&paths), construct).is_none());

    assert_eq!(slot.status().availability, AgentAvailability::Stopped);
    let service = built.lock().unwrap().take().unwrap();
    let runtime = slot.take_runtime().expect("the start built a runtime");
    let refused = runtime
        .block_on(async move {
            service
                .create_session(AgentSessionMode::Default, None)
                .await
        })
        .unwrap();
    runtime.shutdown(Duration::from_secs(2)).unwrap();
    assert_eq!(refused.unwrap_err(), "Agent service is shutting down");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

pub(super) fn storage_of(paths: &TestPaths) -> AgentStoragePaths {
    AgentStoragePaths::under(paths.data_dir.join("agent-test"))
}

/// A stdio MCP server that never answers `initialize`: its connect lasts
/// its connect and initialize timeouts, `timeout_seconds` each.
#[cfg(unix)]
fn silent_mcp_server(timeout_seconds: u64) -> hanafubuki_agent_mcp::McpServerConfig {
    hanafubuki_agent_mcp::McpServerConfig {
        id: hanafubuki_agent_mcp::McpServerId::parse("silent").unwrap(),
        enabled: true,
        required: false,
        protocol_mode: hanafubuki_agent_mcp::McpProtocolMode::Legacy,
        transport: hanafubuki_agent_mcp::McpTransportConfig::Stdio(
            hanafubuki_agent_mcp::McpStdioConfig {
                program: "sleep".into(),
                argv: vec!["300".into()],
                cwd: None,
                public_environment: BTreeMap::new(),
                credential_environment: BTreeMap::new(),
            },
        ),
        connect_timeout_seconds: timeout_seconds,
        initialize_timeout_seconds: timeout_seconds,
        list_timeout_seconds: 1,
        call_timeout_seconds: 1,
        shutdown_timeout_seconds: 2,
        maximum_concurrent_calls: 1,
        tool_maximum_concurrent_calls: BTreeMap::new(),
        enabled_tools: Default::default(),
        disabled_tools: Default::default(),
        approval_policy: hanafubuki_agent_mcp::McpApprovalPolicy::Ask,
        tool_approval_policies: BTreeMap::new(),
        exposure_policy: hanafubuki_agent_mcp::McpExposurePolicy::Deferred,
        tool_exposure_policies: BTreeMap::new(),
    }
}

#[cfg(unix)]
fn add_mcp_server(paths: &TestPaths, server: hanafubuki_agent_mcp::McpServerConfig) {
    let repository = AgentConfig::new(&paths.config_dir).mcp().clone();
    let revision = repository.load().unwrap().revision;
    repository
        .update(revision, move |settings| {
            settings.servers.push(server);
            Ok(())
        })
        .unwrap();
}

pub(super) fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TEST_WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "{what} did not happen in time");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn a_hung_mcp_connect_does_not_hold_the_start_and_fails_on_its_own_timeout() {
    let paths = test_paths("startup-mcp-hung");
    let storage = storage_of(&paths);
    // Connect and initialize time out after this each: longer than a start
    // takes even on a loaded runner, so a start that returns sooner did not
    // wait for the connect.
    let timeout = Duration::from_secs(10);
    add_mcp_server(&paths, silent_mcp_server(timeout.as_secs()));
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let marker = marker(&paths);

    let started = Instant::now();
    let agent = start(&slot, &marker, unrecovered(&paths, storage)).expect("the Agent runs");

    assert!(
        started.elapsed() < timeout,
        "the start waited for MCP: {:?}",
        started.elapsed()
    );
    assert_eq!(slot.status().availability, AgentAvailability::Running);
    let server = hanafubuki_agent_mcp::McpServerId::parse("silent").unwrap();
    wait_until("the MCP connect failing", || {
        agent
            .test_mcp_status(&server)
            .is_some_and(|status| status.phase == hanafubuki_agent_mcp::McpConnectionPhase::Failed)
    });
    let status = agent.test_mcp_status(&server).unwrap();
    assert_eq!(
        status.last_error.as_deref(),
        Some("MCP operation timed out")
    );
    drop(agent);
    let diagnostics = shutdown_agent(&slot, &marker, AGENT_SHUTDOWN_BUDGET);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[cfg(unix)]
#[test]
fn shutdown_cancels_an_mcp_connect_under_way_and_leaves_no_process() {
    let paths = test_paths("startup-mcp-cancelled");
    let storage = storage_of(&paths);
    // It would connect for two minutes.
    add_mcp_server(&paths, silent_mcp_server(60));
    let executor = hanafubuki_agent_process::executor::ExecutorManager::default();
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let marker = marker(&paths);
    let construct = unrecovered_with(
        &paths,
        storage,
        AgentConfig::new(&paths.config_dir),
        Vec::new(),
    );
    let agent = start_with(
        &slot,
        launcher(&paths, &marker, construct, executor.clone()),
    )
    .expect("the Agent runs");
    wait_until("the MCP process starting", || {
        hanafubuki_agent_process::executor::manager::executor_registry_counts(&executor).0 > 0
    });
    drop(agent);

    let started = Instant::now();
    let diagnostics = shutdown_agent(&slot, &marker, AGENT_SHUTDOWN_BUDGET);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        hanafubuki_agent_process::executor::manager::executor_registry_counts(&executor),
        (0, 0, 0),
        "an MCP process outlived the Agent"
    );
    assert!(slot.take_runtime().is_none(), "the runtime was stopped");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A plugin whose start never returns; `entered` is set once it began.
fn never_starting_plugin(
    entered: Arc<AtomicBool>,
) -> (
    super::host::AgentPluginRow,
    super::host::AgentPluginDescriptor,
) {
    let row = super::host::AgentPluginRow::unconfigured(
        AgentPluginGroup::Capability,
        kardia_core::PluginDef::new(move |_, _| {
            let entered = entered.clone();
            async move {
                entered.store(true, Ordering::SeqCst);
                std::future::pending::<kardia_core::Result<()>>().await
            }
        })
        .name("never-starts"),
    );
    (
        row,
        super::host::AgentPluginDescriptor::new("never-starts", "a test plugin"),
    )
}

#[test]
fn a_start_that_shutdown_cancels_stops_at_once_and_is_torn_down() {
    let paths = test_paths("startup-cancelled");
    let entered = Arc::new(AtomicBool::new(false));
    let executor = hanafubuki_agent_process::executor::ExecutorManager::default();
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let marker = marker(&paths);
    let starting = {
        let (slot, marker) = (slot.clone(), marker.clone());
        let construct = unrecovered_with(
            &paths,
            storage_of(&paths),
            AgentConfig::new(&paths.config_dir),
            vec![never_starting_plugin(entered.clone())],
        );
        let launcher = launcher(&paths, &marker, construct, executor.clone());
        std::thread::spawn(move || start_with(&slot, launcher))
    };
    // The start is in its plugins' stage, where the hung plugin would hold
    // it for the start-up timeout.
    wait_until("the hung plugin starting", || {
        entered.load(Ordering::SeqCst)
    });

    let started = Instant::now();
    let diagnostics = shutdown_agent(&slot, &marker, AGENT_SHUTDOWN_BUDGET);

    // Cancelled, the start retired what it built within its own bound, well
    // before the start-up timeout, so shutdown did not abandon it.
    assert!(
        started.elapsed() < crate::shutdown::START_WAIT,
        "{:?}",
        started.elapsed()
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.worker == "agent-start"),
        "{diagnostics:?}"
    );
    assert!(starting.join().unwrap().is_none());
    assert_eq!(slot.status().availability, AgentAvailability::Stopped);
    assert_eq!(marker.unfinished_starts(), 0);
    assert_eq!(
        hanafubuki_agent_process::executor::manager::executor_registry_counts(&executor),
        (0, 0, 0)
    );
    assert!(slot.take_runtime().is_none(), "the runtime was stopped");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn a_plugin_that_never_stops_does_not_hold_shutdown_past_its_budget() {
    let paths = test_paths("startup-stuck-plugin-shutdown");
    let stuck = super::host::AgentPluginRow::unconfigured(
        AgentPluginGroup::Capability,
        kardia_core::PluginDef::new(|ctx: kardia_core::Context, _| async move {
            ctx.defer(
                "never-stops",
                kardia_core::Disposer::future(std::future::pending::<()>),
            )?;
            Ok(())
        })
        .name("never-stops"),
    );
    let stuck = (
        stuck,
        super::host::AgentPluginDescriptor::new("never-stops", "a test plugin"),
    );
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let marker = marker(&paths);
    let agent = start(
        &slot,
        &marker,
        unrecovered_with(
            &paths,
            storage_of(&paths),
            AgentConfig::new(&paths.config_dir),
            vec![stuck],
        ),
    )
    .expect("the Agent runs");
    drop(agent);

    // Below the plugins' own bound (5 s): only the budget can end the wait
    // for the plugin. The stages after it take a moment, longer on a loaded
    // runner, but nowhere near that bound.
    let budget = Duration::from_secs(1);
    let started = Instant::now();
    let diagnostics = shutdown_agent(&slot, &marker, budget);

    assert!(
        started.elapsed() < budget + Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.worker == "agent-plugins"),
        "{diagnostics:?}"
    );
    assert_eq!(slot.status().availability, AgentAvailability::Stopped);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn saved_settings_reach_the_agent_without_waiting_for_a_reconnect_and_in_order() {
    /// The host's proxy, which the test changes as a settings save would.
    struct SavedProxy(Mutex<AgentProxy>);

    impl NetworkSettingsPort for SavedProxy {
        fn proxy(&self) -> Result<AgentProxy, String> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    let paths = test_paths("startup-settings-follow");
    let config = AgentConfig::new(&paths.config_dir);
    let network = Arc::new(SavedProxy(Mutex::new(AgentProxy::System)));
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let marker = marker(&paths);
    let construct: AgentConstructor = {
        let (paths, network) = (paths.clone(), network.clone());
        Arc::new(move |_runtime, instance| {
            let mut host = test_host(&paths);
            host.network = network.clone();
            AgentApplicationService::new_unrecovered(
                config.clone(),
                host,
                storage_of(&paths),
                skill_roots(&paths),
                instance,
            )
        })
    };
    let agent = start(&slot, &marker, construct).expect("the Agent runs");
    let initial = agent.test_mcp_proxy_policy();

    // An MCP reconnect is under way and does not end.
    let gate = agent.test_mcp_configuration_gate();
    let reconnecting = gate.blocking_lock();
    let started = Instant::now();
    *network.0.lock().unwrap() = AgentProxy::Disabled;
    slot.apply_saved_settings();
    *network.0.lock().unwrap() = AgentProxy::Explicit("http://127.0.0.1:9".to_string());
    slot.apply_saved_settings();
    assert!(started.elapsed() < Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(agent.test_mcp_proxy_policy(), initial);

    // Once it ends, the later change is the one that holds.
    drop(reconnecting);
    let latest =
        hanafubuki_agent_mcp::McpHttpProxyPolicy::Explicit("http://127.0.0.1:9".to_string());
    wait_until("the latest proxy applying", || {
        agent.test_mcp_proxy_policy() == latest
    });
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(agent.test_mcp_proxy_policy(), latest);
    drop(agent);
    assert_eq!(shutdown_agent(&slot, &marker, AGENT_SHUTDOWN_BUDGET), []);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[cfg(unix)]
#[test]
fn the_agents_processes_inherit_the_hosts_overlay_and_follow_its_replacement() {
    let paths = test_paths("startup-child-environment");
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let marker = marker(&paths);
    let agent = DesktopAgent::new(
        slot.clone(),
        marker.clone(),
        std::collections::HashMap::from([(
            "HANAFUBUKI_TEST_INHERITED".to_string(),
            Some("inherited".to_string()),
        )]),
    )
    .unwrap();
    agent
        .replace_child_environment(std::collections::HashMap::from([(
            "HANAFUBUKI_TEST_PROXY".to_string(),
            Some("proxy".to_string()),
        )]))
        .unwrap();
    let executors = Arc::new(Mutex::new(Vec::new()));
    let inner = unrecovered(&paths, storage_of(&paths));
    let seen = executors.clone();
    agent.start(AgentLaunch {
        construct: Arc::new(move |runtime, instance| {
            seen.lock().unwrap().push(instance.executor.clone());
            inner(runtime, instance)
        }),
        process_groups_file: storage_of(&paths).process_groups_file,
    });
    wait_until("the Agent running", || slot.agent().is_ok());

    let executor = executors.lock().unwrap()[0].clone();
    let environment_of = |executor| {
        hanafubuki_agent_process::executor::manager::run_and_wait(
            executor,
            hanafubuki_agent_process::executor::ExecutorStartRequest {
                executor_id: None,
                command: vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "printf '%s %s' \"$HANAFUBUKI_TEST_INHERITED\" \"$HANAFUBUKI_TEST_PROXY\""
                        .to_string(),
                ],
                cwd: None,
                environment_inheritance:
                    hanafubuki_agent_process::executor::EnvironmentInheritance::Inherit,
                env: std::collections::HashMap::new(),
                io_mode: hanafubuki_agent_process::executor::ExecutorIoMode::Pipe,
                stream_stdin: false,
                stream_output: true,
                cols: None,
                rows: None,
                timeout_ms: None,
                output_bytes_cap: None,
            },
            Duration::from_secs(10),
        )
        .unwrap()
        .output
    };
    assert_eq!(environment_of(&executor), "inherited proxy");

    // A proxy change while the Agent runs reaches its next process.
    agent
        .replace_child_environment(std::collections::HashMap::from([(
            "HANAFUBUKI_TEST_PROXY".to_string(),
            Some("changed".to_string()),
        )]))
        .unwrap();
    assert_eq!(environment_of(&executor), "inherited changed");
    let diagnostics = agent.shutdown(AGENT_SHUTDOWN_BUDGET);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
