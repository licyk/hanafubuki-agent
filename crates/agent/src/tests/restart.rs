//! The supervisor: a restart rebuilds the Agent from disk and keeps every
//! session; a call in flight on the old Agent does not finish; a kernel
//! fault restarts the Agent after a backoff, at most three times in the
//! window; safe mode, the history index rebuild, and turning the Agent off
//! and on; the processes an Agent left are swept when it stops.

use crate::test_support::TestPaths;
use crate::AgentConfig;
use hanafubuki_agent_process::executor::ExecutorManager;

use super::startup::{launcher, marker, start_with, storage_of, unrecovered_with, wait_until};
use super::*;
use crate::test_support::test_paths;

fn policy(delay: Duration) -> AgentRestartPolicy {
    AgentRestartPolicy {
        delays: vec![delay; 3],
        window: Duration::from_secs(600),
    }
}

/// A running Agent of a slot with `policy`, built by `construct`.
fn running_agent(
    paths: &TestPaths,
    policy: AgentRestartPolicy,
    construct: AgentConstructor,
    executor: ExecutorManager,
) -> (AgentSlot, AgentStartMarker, AgentHandle) {
    let slot = AgentSlot::with_restart_policy(None, Arc::new(NoopAgentStatusEventSink), policy);
    let marker = marker(paths);
    let agent = start_with(&slot, launcher(paths, &marker, construct, executor))
        .unwrap_or_else(|| panic!("the Agent did not start: {:?}", slot.status()));
    (slot, marker, agent)
}

/// An Agent with a Provider no request reaches, so sessions can be created.
fn plain_agent(
    paths: &TestPaths,
    policy: AgentRestartPolicy,
) -> (AgentSlot, AgentStartMarker, AgentHandle) {
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, "http://127.0.0.1:1/v1", None);
    let construct = unrecovered_with(paths, storage_of(paths), config, Vec::new());
    running_agent(paths, policy, construct, ExecutorManager::default())
}

/// Waits until Agent `generation` runs, and returns it.
fn running(slot: &AgentSlot, generation: u64) -> AgentHandle {
    wait_until(&format!("Agent {generation} running"), || {
        slot.agent()
            .is_ok_and(|agent| agent.generation() == generation)
    });
    slot.agent().unwrap()
}

fn faulted(slot: &AgentSlot) -> AgentAvailability {
    wait_until("the Agent faulting", || {
        matches!(
            slot.status().availability,
            AgentAvailability::Faulted { .. }
        )
    });
    slot.status().availability
}

fn shut_down(slot: &AgentSlot, marker: &AgentStartMarker, paths: &TestPaths) {
    let diagnostics = shutdown_agent(slot, marker, AGENT_SHUTDOWN_BUDGET);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(&paths.app_data_dir);
}

fn create_session(agent: &AgentHandle) -> AgentSessionId {
    let creating = agent.clone();
    agent
        .runtime()
        .block_on(async move { creating.create_session(AgentSessionMode::Default).await })
        .unwrap()
        .unwrap()
        .id
}

#[test]
fn a_restart_keeps_every_session_and_its_history() {
    let io = tokio::runtime::Runtime::new().unwrap();
    let server = io.block_on(FakeServer::start(
        (0..4)
            .map(|_| (assistant_events("Restarted fine."), Duration::ZERO))
            .collect(),
    ));
    let paths = test_paths("restart-sessions");
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, &server.base_url, None);
    let construct = unrecovered_with(&paths, storage_of(&paths), config, Vec::new());
    let (slot, marker, agent) = running_agent(
        &paths,
        AgentRestartPolicy::default(),
        construct,
        ExecutorManager::default(),
    );
    let talked = create_session(&agent);
    let empty = create_session(&agent);
    let turning = agent.clone();
    let outcome = agent
        .runtime()
        .block_on(async move { turning.start_turn(talked, "hello".into()).await })
        .unwrap()
        .unwrap();
    assert_eq!(outcome.final_answer, "Restarted fine.");
    let before = [talked, empty].map(|session| agent.open_session(session, 75).unwrap());
    drop(agent);

    slot.restart(AgentStartOptions::default()).unwrap();

    let agent = running(&slot, 2);
    assert!(slot.status().quarantined_sessions.is_empty());
    for (session, before) in [talked, empty].into_iter().zip(before) {
        let after = agent.open_session(session, 75).unwrap();
        assert_eq!(after.transcript, before.transcript);
        assert_eq!(after.runtime.revision, before.runtime.revision);
    }
    drop(agent);
    shut_down(&slot, &marker, &paths);
}

#[test]
fn a_call_in_flight_on_the_old_agent_does_not_finish() {
    let paths = test_paths("restart-in-flight");
    let (slot, marker, agent) = plain_agent(&paths, AgentRestartPolicy::default());
    let in_flight = agent.runtime().handle().spawn(std::future::pending::<()>());
    drop(agent);

    slot.restart(AgentStartOptions::default()).unwrap();
    let _ = running(&slot, 2);

    let dropped = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(in_flight)
        .unwrap_err();
    assert_eq!(
        RuntimeTaskFailure::from(dropped),
        RuntimeTaskFailure::Stopped
    );
    assert_eq!(
        slot.interrupted(1, "agent_start_turn").as_deref(),
        Some("the Agent restarted; agent_start_turn did not finish")
    );
    assert_eq!(slot.interrupted(2, "agent_start_turn"), None);
    shut_down(&slot, &marker, &paths);
}

#[test]
fn a_kernel_fault_stops_the_agent_and_restarts_it_after_a_backoff() {
    let paths = test_paths("restart-fault");
    let (slot, marker, agent) = plain_agent(&paths, policy(Duration::from_millis(500)));

    agent.test_panic_in_agent_worker();
    drop(agent);

    match faulted(&slot) {
        AgentAvailability::Faulted {
            reason,
            restarts_in_window,
            next_restart_at,
            ..
        } => {
            assert_eq!(
                reason,
                "worker agent-test-worker panicked: an Agent worker panicked"
            );
            assert_eq!(restarts_in_window, 0);
            assert!(next_restart_at.is_some());
        }
        other => panic!("not faulted: {other:?}"),
    }
    assert!(slot
        .agent()
        .err()
        .unwrap()
        .to_string()
        .contains("restarts shortly"));
    assert_eq!(
        slot.interrupted(1, "agent_list_skills").as_deref(),
        Some("the Agent restarted; agent_list_skills did not finish")
    );
    let agent = running(&slot, 2);
    assert_eq!(slot.status().availability, AgentAvailability::Running);
    // A stale fault of the stopped Agent does not touch the new one.
    slot.report_kernel_fault(1, "turn", "late");
    assert_eq!(slot.agent().unwrap().generation(), 2);
    drop(agent);
    shut_down(&slot, &marker, &paths);
}

#[test]
fn three_automatic_restarts_use_up_the_window_until_the_user_restarts() {
    let paths = test_paths("restart-window");
    // The first delay is also the one the fault after the user's restart
    // waits, which the test must see as faulted before it restarts again.
    let policy = AgentRestartPolicy {
        delays: vec![
            Duration::from_millis(500),
            Duration::from_millis(20),
            Duration::from_millis(20),
        ],
        window: Duration::from_secs(600),
    };
    let (slot, marker, mut agent) = plain_agent(&paths, policy);
    for restarted in 0..3 {
        agent.test_panic_in_agent_worker();
        agent = running(&slot, 2 + restarted);
    }

    agent.test_panic_in_agent_worker();
    drop(agent);

    let AgentAvailability::Faulted {
        restarts_in_window,
        next_restart_at,
        ..
    } = faulted(&slot)
    else {
        unreachable!("checked to be faulted");
    };
    assert_eq!(restarts_in_window, 3);
    assert_eq!(next_restart_at, None);
    std::thread::sleep(Duration::from_millis(300));
    assert!(matches!(
        slot.status().availability,
        AgentAvailability::Faulted {
            next_restart_at: None,
            ..
        }
    ));
    assert_eq!(
        slot.interrupted(4, "agent_start_turn").as_deref(),
        Some("the Agent stopped after an internal error; agent_start_turn did not finish")
    );

    // The user's restart empties the window: the next fault restarts again.
    slot.restart(AgentStartOptions::default()).unwrap();
    let agent = running(&slot, 5);
    agent.test_panic_in_agent_worker();
    drop(agent);
    let AgentAvailability::Faulted {
        restarts_in_window,
        next_restart_at,
        ..
    } = faulted(&slot)
    else {
        unreachable!("checked to be faulted");
    };
    assert_eq!(restarts_in_window, 0);
    assert!(next_restart_at.is_some());
    let _ = running(&slot, 6);
    shut_down(&slot, &marker, &paths);
}

#[test]
fn safe_mode_starts_only_the_core_and_keeps_the_saved_plugin_setting() {
    let paths = test_paths("restart-safe-mode");
    let config = AgentConfig::new(&paths.config_dir);
    let construct = unrecovered_with(&paths, storage_of(&paths), config.clone(), Vec::new());
    let (slot, marker, agent) = running_agent(
        &paths,
        AgentRestartPolicy::default(),
        construct,
        ExecutorManager::default(),
    );
    assert!(!slot.status().safe_mode);
    drop(agent);

    slot.restart(AgentStartOptions {
        safe_mode: true,
        ..AgentStartOptions::default()
    })
    .unwrap();

    let agent = running(&slot, 2);
    assert!(slot.status().safe_mode);
    let plugins = agent.plugin_status();
    assert!(plugins
        .iter()
        .any(|plugin| plugin.group != AgentPluginGroup::Core));
    for plugin in &plugins {
        if plugin.group == AgentPluginGroup::Core {
            assert_eq!(plugin.state, AgentPluginState::Active, "{plugin:?}");
        } else {
            assert_eq!(plugin.state, AgentPluginState::Disabled, "{plugin:?}");
            assert!(!plugin.enabled);
        }
    }
    let name = plugins
        .iter()
        .find(|plugin| plugin.group != AgentPluginGroup::Core)
        .unwrap()
        .name
        .clone();
    let enabling = agent.clone();
    assert!(agent
        .runtime()
        .block_on(async move { enabling.set_plugin_enabled(name, true).await })
        .unwrap()
        .unwrap_err()
        .contains("safe mode"));
    let saved = config.settings().unwrap().disabled_plugins;
    assert!(saved.is_empty(), "{saved:?}");
    drop(agent);

    // A normal restart leaves safe mode.
    slot.restart(AgentStartOptions::default()).unwrap();
    let agent = running(&slot, 3);
    assert!(!slot.status().safe_mode);
    assert!(agent.plugin_status().iter().all(|plugin| plugin.enabled));
    drop(agent);
    shut_down(&slot, &marker, &paths);
}

#[test]
fn rebuilding_the_history_index_repairs_what_recovery_quarantined() {
    let paths = test_paths("restart-rebuild-index");
    let (slot, marker, agent) = plain_agent(&paths, AgentRestartPolicy::default());
    let [broken, intact] = [0, 1].map(|_| create_session(&agent));
    drop(agent);
    slot.disable().unwrap();
    wait_until("the Agent turning off", || {
        slot.status().availability
            == AgentAvailability::Disabled {
                reason: AgentDisabledReason::Setting,
            }
    });
    // The index, not the rollout, is wrong: recovery quarantines the session.
    let index = hanafubuki_agent_history::AgentHistoryPaths::under(storage_of(&paths).history_root)
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
    slot.enable().unwrap();
    let agent = running(&slot, 2);
    assert_eq!(
        slot.status()
            .quarantined_sessions
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        vec![broken]
    );
    assert!(agent.open_session(broken, 75).is_err());
    drop(agent);

    slot.restart(AgentStartOptions {
        rebuild_history_index: true,
        ..AgentStartOptions::default()
    })
    .unwrap();

    let agent = running(&slot, 3);
    assert!(slot.status().quarantined_sessions.is_empty());
    assert!(agent.open_session(broken, 75).is_ok());
    assert!(agent.open_session(intact, 75).is_ok());
    drop(agent);
    shut_down(&slot, &marker, &paths);
}

#[test]
fn turning_the_agent_off_stops_it_in_order_and_turning_it_on_starts_it() {
    let paths = test_paths("restart-toggle");
    let (slot, marker, agent) = plain_agent(&paths, AgentRestartPolicy::default());
    let session = create_session(&agent);
    drop(agent);

    slot.disable().unwrap();
    // A restart cannot undo the turn off under way.
    assert_eq!(
        slot.restart(AgentStartOptions::default()),
        Err("the Agent is being turned off".to_string())
    );

    wait_until("the Agent turning off", || {
        slot.status().availability
            == AgentAvailability::Disabled {
                reason: AgentDisabledReason::Setting,
            }
    });
    assert_eq!(
        slot.agent().err().unwrap().to_string(),
        "the Agent is unavailable: it is turned off in Settings"
    );
    assert!(slot.take_runtime().is_none(), "the old runtime was stopped");
    assert!(slot.restart(AgentStartOptions::default()).is_err());

    slot.enable().unwrap();

    let agent = running(&slot, 2);
    assert!(agent.open_session(session, 75).is_ok());
    // Turning it on again does nothing.
    slot.enable().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(slot.agent().unwrap().generation(), 2);
    drop(agent);
    shut_down(&slot, &marker, &paths);
}

#[test]
fn the_modules_stop_turns_the_agent_off_and_waits_until_it_is_off() {
    let paths = test_paths("restart-turn-off");
    let (slot, marker, agent) = plain_agent(&paths, AgentRestartPolicy::default());
    drop(agent);

    slot.turn_off(Instant::now() + TEST_WAIT).unwrap();
    // Off when the stop returns, its runtime stopped.
    assert_eq!(
        slot.status().availability,
        AgentAvailability::Disabled {
            reason: AgentDisabledReason::Setting,
        }
    );
    assert!(slot.take_runtime().is_none(), "the old runtime was stopped");
    // Already off, the stop has nothing to wait for.
    slot.turn_off(Instant::now()).unwrap();

    // The module mounted again turns it on.
    slot.enable().unwrap();
    drop(running(&slot, 2));
    shut_down(&slot, &marker, &paths);
    // Shutdown stopped the Agent first: the module's later stop finds the
    // slot closed and returns at once.
    let started = Instant::now();
    slot.turn_off(Instant::now() + TEST_WAIT).unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(slot.status().availability, AgentAvailability::Stopped);
}

#[test]
fn an_agent_stopped_by_a_crash_loop_starts_when_its_module_is_turned_off_and_on() {
    let paths = test_paths("restart-crash-loop-on");
    let marker = marker(&paths);
    for _ in 0..CRASH_LOOP_STARTS {
        marker.record_attempt().unwrap();
    }
    let slot = AgentSlot::new(
        marker.disabled_reason(true),
        Arc::new(NoopAgentStatusEventSink),
    );
    assert_eq!(
        slot.status().availability,
        AgentAvailability::Disabled {
            reason: AgentDisabledReason::CrashLoop
        }
    );
    let construct = unrecovered_with(
        &paths,
        storage_of(&paths),
        AgentConfig::new(&paths.config_dir),
        Vec::new(),
    );
    // Composition hands over the launch; a slot that is off does not start.
    assert!(start_with(
        &slot,
        launcher(&paths, &marker, construct, ExecutorManager::default())
    )
    .is_none());

    // Its module mounting leaves a crash-loop stop alone.
    slot.enable().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        slot.status().availability,
        AgentAvailability::Disabled {
            reason: AgentDisabledReason::CrashLoop
        }
    );
    // Turned off and on, it starts.
    slot.turn_off(Instant::now() + TEST_WAIT).unwrap();
    slot.enable().unwrap();

    let _ = running(&slot, 1);
    assert_eq!(marker.disabled_reason(true), None);
    shut_down(&slot, &marker, &paths);
}

#[cfg(unix)]
#[test]
fn a_stopping_agent_sweeps_the_processes_it_left_running() {
    let paths = test_paths("restart-sweep");
    let executor = ExecutorManager::default();
    let executors = Arc::new(Mutex::new(Vec::<ExecutorManager>::new()));
    let inner = unrecovered_with(
        &paths,
        storage_of(&paths),
        AgentConfig::new(&paths.config_dir),
        Vec::new(),
    );
    let seen = executors.clone();
    let construct: AgentConstructor = Arc::new(move |runtime, instance| {
        seen.lock().unwrap().push(instance.executor.clone());
        inner(runtime, instance)
    });
    let (slot, marker, agent) = running_agent(
        &paths,
        AgentRestartPolicy::default(),
        construct,
        executor.clone(),
    );
    drop(agent);
    // A process of the Agent that nothing in it stops any more, in the
    // instance's owner group of the Agent's Executor.
    let instance_executor = executors.lock().unwrap()[0].clone();
    hanafubuki_agent_process::executor::manager::start_executor(
        &instance_executor,
        hanafubuki_agent_process::executor::ExecutorStartRequest {
            executor_id: None,
            command: vec!["sleep".to_string(), "30".to_string()],
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
    )
    .unwrap();
    #[cfg(target_os = "linux")]
    assert!(storage_of(&paths).process_groups_file.exists());

    slot.restart(AgentStartOptions::default()).unwrap();

    let _ = running(&slot, 2);
    assert_eq!(
        hanafubuki_agent_process::executor::manager::executor_registry_counts(&executor),
        (0, 0, 0),
        "a process of the old Agent outlived it"
    );
    #[cfg(target_os = "linux")]
    wait_until("the ledger forgetting the ended process", || {
        !storage_of(&paths).process_groups_file.exists()
    });
    shut_down(&slot, &marker, &paths);
}
