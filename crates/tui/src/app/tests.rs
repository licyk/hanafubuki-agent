//! The bridge end to end: a scripted model behind the standalone Agent,
//! its events through the bridge into the chat widget, and the chat drawn
//! on a vt100 screen.

use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::{
    assistant_events, configure_test_provider, tool_events, FakeServer, MemorySecretStore,
};
use hanafubuki_agent::{
    AgentConfig, AgentDirs, AgentHandle, AgentRuntime, AgentRuntimeHost, AgentRuntimeOptions,
    AgentSessionMode, AgentStartOptions, ConfigOverrides, AGENT_SHUTDOWN_BUDGET,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::App;
use crate::agent_bridge::{AgentBridge, StatusSink};
use crate::app_event::{AppEvent, AppEventSender};
use crate::chatwidget::{ChatWidget, ChatWidgetInit};
use crate::custom_terminal::Terminal;
use crate::test_support::VT100Backend;
use crate::tui::{FrameRequester, Tui};

const WIDTH: u16 = 100;
const HEIGHT: u16 = 40;
const DEADLINE: Duration = Duration::from_secs(60);

/// The standalone Agent over a temporary home, with the scripted model as
/// its one Provider, and the interface on a vt100 screen.
struct Harness {
    _home: tempfile::TempDir,
    _model_runtime: tokio::runtime::Runtime,
    server: FakeServer,
    runtime: AgentRuntime,
    app: App,
    tui: Tui<VT100Backend>,
    events: Receiver<AppEvent>,
}

impl Harness {
    fn start(responses: Vec<Vec<u8>>) -> Self {
        let model_runtime = tokio::runtime::Runtime::new().unwrap();
        let server = model_runtime.block_on(FakeServer::start(
            responses
                .into_iter()
                .map(|body| (body, Duration::ZERO))
                .collect(),
        ));
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("project");
        std::fs::create_dir_all(cwd.join(".git")).unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "disabledPlugins = [\"titles\"]\n",
        )
        .unwrap();
        configure_test_provider(&AgentConfig::new(home.path()), &server.base_url, None);

        let (sender, events) = channel();
        let sender = AppEventSender::new(sender);
        let runtime = AgentRuntime::standalone(
            AgentRuntimeOptions {
                dirs: AgentDirs {
                    config_dir: home.path().to_path_buf(),
                    data_dir: home.path().join("data"),
                },
                bundled_skills: None,
                cwd,
                add_dirs: Vec::new(),
                profile: None,
                overrides: ConfigOverrides::default(),
                start: AgentStartOptions::default(),
            },
            AgentRuntimeHost {
                secrets: Some(Arc::new(MemorySecretStore::default())),
                status: Arc::new(StatusSink(sender.clone())),
                ..AgentRuntimeHost::default()
            },
        )
        .unwrap();
        runtime.start();
        let agent = runtime.wait_until_started(DEADLINE).unwrap();
        let session = session(&agent);

        let chat_widget = ChatWidget::new(ChatWidgetInit {
            app_event_tx: sender.clone(),
            frame_requester: FrameRequester::new(sender.clone()),
            enhanced_keys_supported: false,
            model: session.1,
            directory: "/project".to_string(),
        });
        let bridge = AgentBridge::new(agent, runtime.slot().clone(), session.0, sender.clone());
        let mut terminal = Terminal::with_options(VT100Backend::new(WIDTH, HEIGHT)).unwrap();
        terminal.set_viewport_area(ratatui::layout::Rect::new(0, HEIGHT - 1, WIDTH, 1));
        Self {
            _home: home,
            _model_runtime: model_runtime,
            server,
            runtime,
            app: App::new(chat_widget, bridge, sender),
            tui: Tui::new(terminal),
            events,
        }
    }

    fn type_and_submit(&mut self, text: &str) {
        for character in text.chars() {
            self.key(KeyCode::Char(character));
        }
        // Keys this fast read as a paste, whose Enter is a newline: let
        // the burst end first, as a person's pause does.
        std::thread::sleep(Duration::from_millis(100));
        self.app.draw(&mut self.tui).unwrap();
        self.key(KeyCode::Enter);
    }

    fn key(&mut self, code: KeyCode) {
        let event = AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE));
        assert!(self.app.handle_event(&mut self.tui, event));
    }

    /// Runs the app loop's steps until the screen shows `text`.
    fn pump_until_screen_shows(&mut self, text: &str) -> String {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.app.draw(&mut self.tui).unwrap();
            let screen = self.screen();
            if screen.contains(text) {
                return screen;
            }
            let wait = deadline.saturating_duration_since(Instant::now());
            assert!(
                !wait.is_zero(),
                "the screen never showed {text:?}:\n{screen}"
            );
            if let Ok(event) = self
                .events
                .recv_timeout(wait.min(Duration::from_millis(100)))
            {
                assert!(self.app.handle_event(&mut self.tui, event));
            }
        }
    }

    fn screen(&self) -> String {
        self.tui.terminal.backend().vt100().screen().contents()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.runtime.shutdown(AGENT_SHUTDOWN_BUDGET);
    }
}

fn session(agent: &AgentHandle) -> (hanafubuki_agent::AgentSessionId, String) {
    let agent = agent.clone();
    let runtime = agent.runtime().clone();
    let session = runtime
        .block_on(async move { agent.create_session(AgentSessionMode::Default).await })
        .unwrap()
        .unwrap();
    (session.id, session.model)
}

fn command_call(command: &str) -> Vec<u8> {
    let arguments = serde_json::json!({
        "cmd": command,
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": 5_000,
        "max_output_bytes": 32_768,
        "workdir": null
    });
    tool_events("exec_command", &arguments.to_string())
}

#[test]
fn a_submitted_prompt_renders_the_models_answer() {
    let mut harness = Harness::start(vec![assistant_events("The answer is 42.")]);
    harness.type_and_submit("What is the answer?");
    let screen = harness.pump_until_screen_shows("The answer is 42.");
    assert!(screen.contains("› What is the answer?"), "{screen}");
    assert!(screen.contains("model:"), "{screen}");
    assert_eq!(harness.server.requests().len(), 1);
}

#[test]
fn an_approved_command_runs_and_the_turn_completes() {
    let mut harness = Harness::start(vec![
        command_call("echo approved-output"),
        assistant_events("The command ran."),
    ]);
    harness.type_and_submit("Run the command.");
    harness.pump_until_screen_shows("Yes, proceed");
    harness.key(KeyCode::Char('y'));
    let screen = harness.pump_until_screen_shows("The command ran.");
    assert!(screen.contains("approved-output"), "{screen}");
    assert!(screen.contains("exec_command"), "{screen}");
}
