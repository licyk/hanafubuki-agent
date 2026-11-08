//! The standalone runtime end to end: started on its own configuration and
//! data directories with the host ports' defaults, it runs a turn in its
//! working directory, holds its data directory against a second runtime, and
//! shuts down cleanly.

use std::path::PathBuf;

use hanafubuki_agent_foundation::paths::ResolvedPath;

use super::*;

const START_TIMEOUT: Duration = Duration::from_secs(30);

struct Standalone {
    _root: tempfile::TempDir,
    home: PathBuf,
    cwd: PathBuf,
    secrets: MemorySecretStore,
}

/// A home (the configuration and data directory) whose provider is
/// `base_url`, and a working directory inside a Git repository.
fn standalone(base_url: &str) -> Standalone {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let repository = root.path().join("repository");
    let cwd = repository.join("src");
    std::fs::create_dir_all(repository.join(".git")).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    configure_test_provider(&AgentConfig::new(&home), base_url, None);
    std::fs::write(
        home.join("config.toml"),
        "permissionMode = \"manual\"\n\n[profiles.ci]\npermissionMode = \"auto_review\"\n",
    )
    .unwrap();
    Standalone {
        _root: root,
        home,
        cwd,
        secrets: MemorySecretStore::default(),
    }
}

impl Standalone {
    fn runtime(&self) -> Result<AgentRuntime, String> {
        AgentRuntime::standalone(
            AgentRuntimeOptions {
                dirs: AgentDirs::resolve(Some(self.home.clone())).unwrap(),
                bundled_skills: None,
                cwd: self.cwd.clone(),
                add_dirs: Vec::new(),
                profile: Some("ci".to_string()),
                overrides: ConfigOverrides::parse(&["logLevel=debug"]).unwrap(),
                start: AgentStartOptions::default(),
            },
            AgentRuntimeHost {
                secrets: Some(Arc::new(self.secrets.clone())),
                ..AgentRuntimeHost::default()
            },
        )
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<AgentEvent>>);

impl AgentEventSink for RecordingSink {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn the_standalone_runtime_runs_a_turn_in_its_working_directory() {
    let model = tokio::runtime::Runtime::new().unwrap();
    // The `titles` plugin's request races the turn's for the responses.
    let server = model.block_on(FakeServer::start(vec![
        (assistant_events("Standalone answer"), Duration::ZERO),
        (assistant_events("Standalone answer"), Duration::ZERO),
    ]));
    let fixture = standalone(&server.base_url);
    let runtime = fixture.runtime().unwrap();
    let settings = runtime.config().settings().unwrap();
    assert_eq!(settings.permission_mode, AgentPermissionMode::AutoReview);
    assert_eq!(
        runtime.config().standalone_settings().unwrap().log_level,
        AgentLogLevel::Debug
    );

    runtime.start();
    let agent = runtime.wait_until_started(START_TIMEOUT).unwrap();
    let sink = Arc::new(RecordingSink::default());
    let session = agent
        .runtime()
        .block_on({
            let agent = agent.clone();
            let sink = sink.clone();
            async move {
                let session = agent
                    .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
                    .await
                    .unwrap();
                agent
                    .submit_turn_with_sink(
                        session.id,
                        AgentTurnRequest {
                            text: "Say something".to_string(),
                            images: Vec::new(),
                            skills: Vec::new(),
                            host: Default::default(),
                        },
                        sink,
                    )
                    .await
                    .unwrap();
                session
            }
        })
        .unwrap();
    let deadline = Instant::now() + START_TIMEOUT;
    let turn = loop {
        let turn = agent.snapshot(session.id).unwrap().turn;
        if let Some(turn) = turn.filter(|turn| turn.phase.is_terminal()) {
            break turn;
        }
        assert!(Instant::now() < deadline, "the turn did not end");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(turn.phase, AgentTurnPhase::Completed, "{turn:?}");
    assert!(!sink.0.lock().unwrap().is_empty());

    // The model saw the working directory as the workspace, and the
    // embedded System Skills were published into the data directory.
    let cwd = ResolvedPath::resolve(&fixture.cwd).unwrap();
    let cwd_text = serde_json::to_string(cwd.native_text()).unwrap();
    let requests = server.requests();
    assert!(
        requests
            .iter()
            .any(|request| String::from_utf8_lossy(&request.body)
                .contains(cwd_text.trim_matches('"'))),
        "no request names {}",
        cwd.native_text()
    );
    assert!(fixture
        .home
        .join("skills/system/skill-creator/SKILL.md")
        .is_file());

    // A second runtime on the same data directory does not start.
    let second = fixture.runtime().unwrap();
    second.start();
    let refused = second.wait_until_started(START_TIMEOUT).err().unwrap();
    assert!(
        refused.to_string().contains(&format!(
            "the Agent data directory {} is in use by another process",
            fixture.home.display()
        )),
        "{refused}"
    );
    assert!(second.shutdown(AGENT_SHUTDOWN_BUDGET).is_empty());

    drop(agent);
    let diagnostics = runtime.shutdown(AGENT_SHUTDOWN_BUDGET);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    // Released with the shutdown.
    DataDirLock::acquire(&fixture.home).unwrap();
}

#[test]
fn a_profile_the_configuration_does_not_have_fails_the_runtime() {
    let fixture = standalone("http://127.0.0.1:9/v1");
    let error = AgentRuntime::standalone(
        AgentRuntimeOptions {
            dirs: AgentDirs::resolve(Some(fixture.home.clone())).unwrap(),
            bundled_skills: None,
            cwd: fixture.cwd.clone(),
            add_dirs: Vec::new(),
            profile: Some("missing".to_string()),
            overrides: ConfigOverrides::default(),
            start: AgentStartOptions::default(),
        },
        AgentRuntimeHost::default(),
    )
    .err()
    .unwrap();
    assert!(error.contains("no profile `missing`"), "{error}");
}
