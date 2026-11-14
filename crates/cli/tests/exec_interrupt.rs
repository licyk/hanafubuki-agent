//! A real Ctrl-C closes delegated work before shutdown and preserves that report.

#![cfg(unix)]

#[path = "support/mod.rs"]
mod support;

use std::io::BufRead;
use std::process::Stdio;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::{
    add_test_provider, assistant_events, tool_events, FakeServer, HOLD,
};
use hanafubuki_agent::{
    AgentConfig, AgentDirs, AgentEvent, AgentEventSink, AgentRuntime, AgentRuntimeHost,
    AgentRuntimeOptions, AgentSessionId, AgentSubagentWake, AgentWakeSource,
};
use serde_json::json;
use support::Home;

struct Discard;
impl AgentEventSink for Discard {
    fn emit(&self, _: AgentEvent) {}
}

#[test]
fn ctrl_c_while_waiting_preserves_closed_by_user_after_reopen() {
    let models = tokio::runtime::Runtime::new().unwrap();
    let spawn = tool_events("spawn_agent", &json!({"task_name":"worker","message":"Wait.","fork_turns":"none","provider":"provider-2","model":"fixture-model"}).to_string());
    let parent = models.block_on(FakeServer::start(vec![
        (spawn, Duration::ZERO),
        (assistant_events("Waiting for the worker."), Duration::ZERO),
        (assistant_events("The worker was closed."), Duration::ZERO),
    ]));
    let worker = models.block_on(FakeServer::start(vec![(
        [HOLD.as_bytes(), &assistant_events("Too late.")].concat(),
        Duration::ZERO,
    )]));
    let home = Home::new().with_provider(&parent.base_url);
    add_test_provider(
        &AgentConfig::new(&home.home),
        "provider-2",
        "Second",
        &worker.base_url,
    );
    let mut process = home
        .command()
        .args(["exec", "Delegate the work."])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = process.stdout.take().unwrap();
    let (ready, waiting) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.unwrap();
            if line.contains("waiting for 1 agent") {
                let _ = ready.send(());
            }
        }
    });
    waiting.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(std::process::Command::new("kill")
        .args(["-INT", &process.id().to_string()])
        .status()
        .unwrap()
        .success());
    support::Run(process.wait_with_output().unwrap()).expect(130);
    reader.join().unwrap();

    let listed = home.run(&["sessions", "list", "--json"]).expect(0);
    let root: serde_json::Value =
        serde_json::from_str(listed.stdout().lines().next().unwrap()).unwrap();
    let root = AgentSessionId::new(root["sessionId"].as_u64().unwrap());
    let runtime = AgentRuntime::standalone(
        AgentRuntimeOptions {
            dirs: AgentDirs::resolve(Some(home.home.clone())).unwrap(),
            bundled_skills: None,
            cwd: home.cwd.clone(),
            add_dirs: Vec::new(),
            profile: None,
            overrides: Default::default(),
            start: Default::default(),
        },
        AgentRuntimeHost::default(),
    )
    .unwrap();
    runtime.start();
    let agent = runtime.wait_until_started(Duration::from_secs(30)).unwrap();
    let cloned = agent.clone();
    let accepted = agent
        .runtime()
        .block_on(async move {
            cloned
                .submit_wake_turn_with_sink(root, Arc::new(Discard))
                .await
        })
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(accepted.events.len(), 1);
    assert!(matches!(
        accepted.events[0].source,
        AgentWakeSource::Subagent {
            event: AgentSubagentWake::ClosedByUser,
            ..
        }
    ));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !agent
        .snapshot(root)
        .unwrap()
        .turn
        .is_some_and(|turn| turn.phase.is_terminal())
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(parent.requests().len(), 3);
    assert!(runtime.shutdown(Duration::from_secs(10)).is_empty());
}
