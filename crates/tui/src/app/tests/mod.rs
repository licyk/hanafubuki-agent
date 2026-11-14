//! The bridge end to end: a scripted model behind the standalone Agent,
//! its events through the bridge into the chat widget, and the chat drawn
//! on a vt100 screen.

use crate::appearance::Appearance;
use hanafubuki_agent::AgentTuiPreferences;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::{
    assistant_events, configure_test_provider, reasoning_answer_events, reasoning_tool_events,
    tool_events, FakeServer, MemorySecretStore,
};
use hanafubuki_agent::{
    AgentConfig, AgentDirs, AgentRuntime, AgentRuntimeHost, AgentRuntimeOptions, AgentSessionMode,
    AgentStartOptions, ConfigOverrides, AGENT_SHUTDOWN_BUDGET,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::App;
use crate::agent_bridge::{AgentBridge, AgentReply, StatusSink};
use crate::app_event::{AppEvent, AppEventSender};
use crate::chatwidget::{ChatWidget, ChatWidgetInit};
use crate::custom_terminal::Terminal;
use crate::history_cell::HistoryRenderMode;
use crate::test_support::VT100Backend;
use crate::transcript_mode::TranscriptMode;
use crate::transcript_view::TranscriptHistoryState;
use crate::tui::{FrameRequester, Tui};
use crate::tui_state::TuiState;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 40;
const DEADLINE: Duration = Duration::from_secs(60);

/// The standalone Agent over a temporary home, with the scripted model as
/// its one Provider, and the interface on a vt100 screen, started as
/// `hanafubuki-agent` starts it: with no session.
struct Harness {
    home: tempfile::TempDir,
    _model_runtime: tokio::runtime::Runtime,
    server: FakeServer,
    runtime: Arc<AgentRuntime>,
    app: App,
    tui: Tui<VT100Backend>,
    events: Receiver<AppEvent>,
}

impl Harness {
    fn start(responses: Vec<Vec<u8>>) -> Self {
        Self::start_with(responses, true, TranscriptMode::Terminal)
    }

    /// The interface as it starts by default: owning the transcript on the
    /// alternate screen.
    fn owned(responses: Vec<Vec<u8>>) -> Self {
        Self::start_with(responses, true, TranscriptMode::Owned)
    }

    /// `with_provider: false` leaves the configuration with no Provider.
    fn start_with(responses: Vec<Vec<u8>>, with_provider: bool, mode: TranscriptMode) -> Self {
        let responses = responses
            .into_iter()
            .map(|body| (body, Duration::ZERO))
            .collect();
        Self::start_timed(responses, with_provider, mode)
    }

    /// Each response is sent after its delay.
    fn start_timed(
        responses: Vec<(Vec<u8>, Duration)>,
        with_provider: bool,
        mode: TranscriptMode,
    ) -> Self {
        Self::start_configured(responses, with_provider, mode, |_| {})
    }

    /// `configure` changes the configuration before the Agent starts.
    fn start_configured(
        responses: Vec<(Vec<u8>, Duration)>,
        with_provider: bool,
        mode: TranscriptMode,
        configure: impl FnOnce(&AgentConfig),
    ) -> Self {
        Self::start_layered(responses, with_provider, mode, configure, None)
    }

    /// [`Self::start_configured`], under the profile `profile` (`-p`).
    fn start_layered(
        responses: Vec<(Vec<u8>, Duration)>,
        with_provider: bool,
        mode: TranscriptMode,
        configure: impl FnOnce(&AgentConfig),
        profile: Option<&str>,
    ) -> Self {
        let home = tempfile::tempdir().unwrap();
        Self::start_in(home, responses, with_provider, mode, configure, profile)
    }

    /// [`Self::start_layered`] over `home`, which an earlier run may have
    /// used.
    fn start_in(
        home: tempfile::TempDir,
        responses: Vec<(Vec<u8>, Duration)>,
        with_provider: bool,
        mode: TranscriptMode,
        configure: impl FnOnce(&AgentConfig),
        profile: Option<&str>,
    ) -> Self {
        let model_runtime = tokio::runtime::Runtime::new().unwrap();
        let server = model_runtime.block_on(FakeServer::start(responses));
        let cwd = home.path().join("project");
        std::fs::create_dir_all(cwd.join(".git")).unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "disabledPlugins = [\"titles\"]\n",
        )
        .unwrap();
        let config = AgentConfig::new(home.path());
        if with_provider {
            configure_test_provider(&config, &server.base_url, None);
        }
        configure(&config);

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
                profile: profile.map(str::to_string),
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
        let runtime = Arc::new(runtime);
        runtime.start();
        let agent = runtime.wait_until_started(DEADLINE).unwrap();

        let bridge = AgentBridge::new(runtime.clone(), agent, None, None, sender.clone());
        let chat_widget = ChatWidget::new(ChatWidgetInit {
            app_event_tx: sender.clone(),
            frame_requester: FrameRequester::new(sender.clone()),
            enhanced_keys_supported: false,
            directory: "/project".to_string(),
            owned_transcript: mode.is_owned(),
            fullscreen_preference: true,
            model_override: false,
            appearance: Appearance::new(&AgentTuiPreferences::default(), "/project".to_string()).0,
        });
        let (state, _) = TuiState::load(&home.path().join("data"), sender.clone());
        let mut app = App::new(
            chat_widget,
            bridge,
            sender,
            mode,
            HistoryRenderMode::Rich,
            state,
        );
        let mut terminal = Terminal::with_options(VT100Backend::new(WIDTH, HEIGHT)).unwrap();
        terminal.set_viewport_area(ratatui::layout::Rect::new(0, HEIGHT - 1, WIDTH, 1));
        let mut tui = Tui::new(terminal);
        if mode.is_owned() {
            app.enter_owned_screen(&mut tui).unwrap();
        }
        let opened = app.bridge.no_session().unwrap();
        if let Some(update) = app.bridge.reply(AgentReply::SessionOpened(opened)) {
            app.show_update(update);
        }
        Self {
            home,
            _model_runtime: model_runtime,
            server,
            runtime,
            app,
            tui,
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
        self.key_with(code, KeyModifiers::NONE);
    }

    fn key_with(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let event = AppEvent::Key(KeyEvent::new(code, modifiers));
        assert!(self.app.handle_event(&mut self.tui, event));
    }

    /// Runs the app loop's steps until the screen shows `text`.
    fn pump_until_screen_shows(&mut self, text: &str) -> String {
        self.pump_until(&format!("the screen to show {text:?}"), |harness| {
            harness.screen().contains(text)
        });
        self.screen()
    }

    /// Down in the settings center until the selected row is `label`, then
    /// up when it is above.
    fn select_setting(&mut self, label: &str) {
        let selected = format!("› {label}");
        for step in 0..160 {
            self.app.draw(&mut self.tui).unwrap();
            let screen = self.screen();
            if screen
                .lines()
                .any(|line| line.trim_start().starts_with(&selected))
            {
                return;
            }
            self.key(if step < 80 {
                KeyCode::Down
            } else {
                KeyCode::Up
            });
        }
        panic!("no row {label}:\n{}", self.screen());
    }

    /// The text row `label` of the settings center edited to `text`, and
    /// saved when it saves at once.
    fn edit_setting(&mut self, label: &str, text: &str) {
        self.select_setting(label);
        self.key(KeyCode::Enter);
        for character in text.chars() {
            self.key(KeyCode::Char(character));
        }
        self.key(KeyCode::Enter);
        self.pump_until("the change to be saved", |harness| {
            !harness.screen().contains("Saving…")
        });
    }

    /// Waits for the settings center's page to close (its views done), then
    /// closes the center, a page at a time.
    fn leave_settings(&mut self) {
        self.pump_until("the page to close", |harness| {
            harness
                .app
                .workspace
                .settings()
                .is_some_and(|center| center.hosted_command().is_none())
        });
        for _ in 0..8 {
            if self.app.workspace.settings().is_none() {
                return;
            }
            self.key(KeyCode::Esc);
        }
        panic!("the center stays open:\n{}", self.screen());
    }

    /// Runs the app loop's steps until `done` holds.
    fn pump_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.app.draw(&mut self.tui).unwrap();
            if done(self) {
                return;
            }
            let wait = deadline.saturating_duration_since(Instant::now());
            assert!(
                !wait.is_zero(),
                "waited in vain for {what}:\n{}\n-- transcript --\n{}",
                self.screen(),
                self.transcript()
            );
            if let Ok(event) = self
                .events
                .recv_timeout(wait.min(Duration::from_millis(100)))
            {
                assert!(self.app.handle_event(&mut self.tui, event));
            }
        }
    }

    /// Waits until the chat has seen the session's turn end, so the
    /// commands a running turn forbids run.
    fn pump_until_turn_ends(&mut self) {
        self.pump_until("the turn to end", |harness| {
            !harness.app.chat_widget.is_task_running()
        });
        // What the turn's end sent (its answer's last rows and their
        // consolidation) is taken too, as the loop would before a key.
        while let Ok(event) = self.events.try_recv() {
            assert!(self.app.handle_event(&mut self.tui, event));
        }
        self.app.draw(&mut self.tui).unwrap();
    }

    fn screen(&self) -> String {
        self.tui.terminal.backend().vt100().screen().contents()
    }

    /// The text of the cells the session's transcript holds.
    fn transcript(&self) -> String {
        self.app
            .transcript_cells
            .iter()
            .flat_map(|cell| cell.display_lines(WIDTH))
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.runtime.shutdown(AGENT_SHUTDOWN_BUDGET);
    }
}

fn command_call(command: &str) -> Vec<u8> {
    tool_events("exec_command", &command_arguments(command))
}

fn command_arguments(command: &str) -> String {
    serde_json::json!({
        "cmd": command,
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768,
        "workdir": null
    })
    .to_string()
}

#[test]
fn the_interface_starts_without_a_session_and_the_first_message_creates_it() {
    let mut harness = Harness::start(vec![assistant_events("The answer is 42.")]);
    // The header names the model a session will get.
    harness.pump_until_screen_shows("fixture-model");
    assert_eq!(harness.app.bridge.session_id(), None);
    harness.type_and_submit("What is the answer?");
    harness.pump_until_screen_shows("The answer is 42.");
    let session_id = harness.app.bridge.session_id().unwrap();
    let agent = harness.runtime.slot().agent().unwrap();
    assert_eq!(agent.snapshot(session_id).unwrap().model, "fixture-model");
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
    harness.pump_until_screen_shows("The command ran.");
    // What the call shows is read once it finished, off the app loop.
    harness.pump_until("the output", |harness| {
        has_output_row(&harness.screen(), "approved-output")
    });
    assert!(harness.screen().contains("• Ran echo approved-output"));
}

#[test]
fn status_reports_the_session_through_the_facade() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/status");
    let screen = harness.pump_until_screen_shows("Permissions:");
    assert!(screen.contains("fixture-model"), "{screen}");
    assert!(screen.contains("Fixture Provider"), "{screen}");
    assert!(screen.contains("manual"), "{screen}");
    assert!(screen.contains("project"), "{screen}");
}

#[test]
fn mode_switches_the_session_to_plan_mode() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/mode");
    harness.pump_until_screen_shows("Select Mode");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Mode set to plan");
    let agent = harness.runtime.slot().agent().unwrap();
    let session = agent
        .snapshot(harness.app.bridge.session_id().unwrap())
        .unwrap();
    assert_eq!(session.mode, AgentSessionMode::Plan);
}

#[test]
fn a_finished_session_is_resumed_from_the_picker_and_its_transcript_replayed() {
    let mut harness = Harness::start(vec![assistant_events("The answer is 42.")]);
    harness.type_and_submit("What is the answer?");
    harness.pump_until_screen_shows("The answer is 42.");
    harness.pump_until_turn_ends();
    let finished = harness.app.bridge.session_id();

    harness.type_and_submit("/new");
    harness.pump_until("no session", |harness| {
        harness.app.bridge.session_id().is_none()
    });
    assert!(!harness.transcript().contains("The answer is 42."));

    harness.type_and_submit("/resume");
    let picker = harness.pump_until_screen_shows("Resume a previous session");
    // `/new` created no session, so only the finished one is listed.
    assert!(picker.contains("What is the answer?"), "{picker}");
    harness.key(KeyCode::Enter);
    harness.pump_until("the resumed transcript", |harness| {
        harness.transcript().contains("The answer is 42.")
    });
    assert_eq!(harness.app.bridge.session_id(), finished);
    let transcript = harness.transcript();
    assert!(transcript.contains("› What is the answer?"), "{transcript}");
    assert!(transcript.contains("model:"), "{transcript}");
}

#[test]
fn a_resumed_session_replays_its_tool_calls_with_their_output() {
    let mut harness = Harness::start(vec![
        command_call("echo replayed-output"),
        assistant_events("The command ran."),
    ]);
    harness.type_and_submit("Run the command.");
    harness.pump_until_screen_shows("Yes, proceed");
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("The command ran.");
    harness.pump_until_turn_ends();

    harness.type_and_submit("/new");
    harness.pump_until("no session", |harness| {
        harness.app.bridge.session_id().is_none()
    });
    harness.type_and_submit("/resume");
    let picker = harness.pump_until_screen_shows("Resume a previous session");
    assert!(picker.contains("Run the command."), "{picker}");
    harness.key(KeyCode::Enter);
    harness.pump_until("the resumed transcript", |harness| {
        harness.transcript().contains("The command ran.")
    });
    let transcript = harness.transcript();
    assert!(
        transcript.contains("• Ran echo replayed-output"),
        "{transcript}"
    );
    assert!(
        has_output_row(&transcript, "replayed-output"),
        "{transcript}"
    );
}

#[test]
fn a_thinking_turn_writes_its_thought_before_the_answer_and_the_pager_shows_the_text() {
    let mut harness = Harness::start(vec![reasoning_answer_events(
        "Weighing the **question**.",
        "The answer is 42.",
    )]);
    harness.type_and_submit("What is the answer?");
    let screen = harness.pump_until_screen_shows("The answer is 42.");
    let thought = screen.find("• Thought ·").expect(&screen);
    assert!(
        thought < screen.find("The answer is 42.").unwrap(),
        "{screen}"
    );
    assert!(!screen.contains("Weighing the question."), "{screen}");
    harness.pump_until_turn_ends();

    // The transcript opens compact; a second Ctrl-T shows the details.
    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    let pager = harness.pump_until_screen_shows("T R A N S C R I P T");
    assert!(pager.contains("• Thought ·"), "{pager}");
    assert!(!pager.contains("Weighing the question."), "{pager}");
    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("Weighing the question.");
}

#[test]
fn a_resumed_session_replays_its_thinking() {
    let mut harness = Harness::start(vec![reasoning_answer_events(
        "Weighing the question.",
        "The answer is 42.",
    )]);
    harness.type_and_submit("What is the answer?");
    harness.pump_until_screen_shows("The answer is 42.");
    harness.pump_until_turn_ends();

    harness.type_and_submit("/new");
    harness.pump_until("no session", |harness| {
        harness.app.bridge.session_id().is_none()
    });
    harness.type_and_submit("/resume");
    harness.pump_until_screen_shows("Resume a previous session");
    harness.key(KeyCode::Enter);
    harness.pump_until("the resumed transcript", |harness| {
        harness.transcript().contains("The answer is 42.")
    });
    let transcript = harness.transcript();
    let thought = transcript.find("• Thought ·").expect(&transcript);
    assert!(
        thought < transcript.find("The answer is 42.").unwrap(),
        "{transcript}"
    );
    // Named by its history item, not by the run's reasoning id.
    let ids: Vec<String> = harness
        .app
        .transcript_cells
        .iter()
        .flat_map(|cell| cell.activity_ids())
        .filter(|id| id.starts_with("thinking:"))
        .collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert!(ids[0].starts_with("thinking:h"), "{ids:?}");
}

#[test]
fn each_step_writes_its_thought_before_its_call_or_answer() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(
            vec![
                reasoning_tool_events(
                    "Run it first.",
                    "exec_command",
                    &command_arguments("echo thought-output"),
                ),
                reasoning_answer_events("It ran.", "The command ran."),
            ],
            true,
            mode,
        );
        harness.type_and_submit("Run the command.");
        harness.pump_until_screen_shows("Yes, proceed");
        harness.key(KeyCode::Char('y'));
        harness.pump_until_screen_shows("The command ran.");
        harness.pump_until_turn_ends();
        let transcript = harness.transcript();
        let order = [
            "• Thought ·",
            "echo thought-output",
            "• Thought ·",
            "The command ran.",
        ];
        let mut from = 0;
        for text in order {
            let at = transcript[from..]
                .find(text)
                .unwrap_or_else(|| panic!("{text:?} not in order:\n{transcript}"));
            from += at + text.len();
        }
    }
}

#[test]
fn in_fullscreen_a_tool_call_is_one_entry_in_every_frame_from_its_start() {
    let mut harness = Harness::owned(vec![
        command_call("echo in-place-output"),
        assistant_events("The command ran."),
    ]);
    harness.type_and_submit("Run the command.");
    harness.pump_until_screen_shows("Yes, proceed");
    harness.key(KeyCode::Char('y'));
    // The approval's decision names the command too.
    let shows_call = |text: &str| {
        text.contains("Running echo in-place-output") || text.contains("Ran echo in-place-output")
    };
    let calls = |harness: &Harness| {
        harness
            .app
            .transcript_cells
            .iter()
            .filter(|cell| shows_call(&cell_text(cell.as_ref())))
            .count()
    };
    let started = std::cell::Cell::new(false);
    harness.pump_until("the answer", |harness| {
        let screen = harness.screen();
        if started.get() {
            assert!(shows_call(&screen), "a frame lost the call:\n{screen}");
        }
        started.set(shows_call(&screen));
        assert!(calls(harness) <= 1, "{}", harness.transcript());
        screen.contains("The command ran.")
    });
    harness.pump_until("the output", |harness| {
        has_output_row(&harness.screen(), "in-place-output")
    });
    assert!(!harness.screen().contains("Running echo"));
    assert_eq!(calls(&harness), 1);
}

/// Whether a row of `screen` is `output` alone, as a call's output row
/// is, under the header that names the command.
fn has_output_row(screen: &str, output: &str) -> bool {
    screen
        .lines()
        .any(|row| row.trim().trim_start_matches("└").trim() == output)
}

fn cell_text(cell: &dyn crate::history_cell::HistoryCell) -> String {
    cell.display_lines(WIDTH)
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
        .collect()
}

impl Harness {
    /// The last `rows` rows of the screen.
    fn bottom_rows(&self, rows: usize) -> String {
        let screen = self.screen();
        let lines: Vec<&str> = screen.lines().collect();
        lines[lines.len().saturating_sub(rows)..].join("\n")
    }

    fn config_file(&self) -> String {
        std::fs::read_to_string(self.home.path().join("config.toml")).unwrap()
    }
}

#[test]
fn owned_mode_draws_the_conversation_on_the_alternate_screen() {
    let mut harness = Harness::owned(vec![assistant_events("The answer is 42.")]);
    harness.pump_until_screen_shows("fixture-model");
    assert!(harness
        .tui
        .terminal
        .backend()
        .vt100()
        .screen()
        .alternate_screen());
    harness.type_and_submit("What is the answer?");
    let screen = harness.pump_until_screen_shows("The answer is 42.");
    assert!(screen.contains("› What is the answer?"), "{screen}");
    // The composer stays at the bottom, below the conversation.
    let composer = screen.find("Ask the Agent to do anything").expect(&screen);
    assert!(
        composer > screen.find("The answer is 42.").unwrap(),
        "{screen}"
    );
}

#[test]
fn a_scrolled_owned_transcript_offers_the_way_back_to_the_latest() {
    let mut harness = Harness::owned(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    for note in 0..60 {
        harness
            .app
            .chat_widget
            .add_info_message(format!("note {note:02}"));
    }
    harness.pump_until_screen_shows("note 59");
    harness.key(KeyCode::PageUp);
    harness.app.draw(&mut harness.tui).unwrap();
    let scrolled = harness.screen();
    assert!(!scrolled.contains("note 59"), "{scrolled}");
    insta::assert_snapshot!(harness.bottom_rows(6));

    // Esc returns to the latest output, and so does Enter on an empty
    // composer.
    harness.key(KeyCode::Esc);
    let latest = harness.pump_until_screen_shows("note 59");
    assert!(!latest.contains("Back to bottom"), "{latest}");
    harness.key(KeyCode::PageUp);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.screen().contains("Back to bottom"));
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("note 59");
    assert_eq!(harness.app.bridge.session_id(), None);
}

#[test]
fn ctrl_t_shows_the_owned_transcript_in_detail_in_place() {
    let mut harness = Harness::owned(vec![reasoning_answer_events(
        "Weighing the **question**.",
        "The answer is 42.",
    )]);
    harness.type_and_submit("What is the answer?");
    let compact = harness.pump_until_screen_shows("The answer is 42.");
    harness.pump_until_turn_ends();
    assert!(compact.contains("• Thought ·"), "{compact}");
    assert!(!compact.contains("Weighing the question."), "{compact}");

    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    let detailed = harness.pump_until_screen_shows("Weighing the question.");
    assert!(harness.app.transcript_view.is_detailed());
    // Still the conversation's screen, with the composer below.
    assert!(
        detailed.contains("Ask the Agent to do anything"),
        "{detailed}"
    );

    harness.key(KeyCode::Esc);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.app.transcript_view.is_detailed());
    assert!(!harness.screen().contains("Weighing the question."));
    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert!(!harness.app.transcript_view.is_detailed());
}

#[test]
fn the_terminal_transcript_overlay_holds_the_tail_until_it_closes() {
    let mut harness = Harness::start(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("T R A N S C R I P T");
    harness
        .app
        .chat_widget
        .add_info_message("written while open".to_string());
    harness.pump_until_screen_shows("written while open");
    let in_tail = |harness: &Harness| {
        harness
            .app
            .tail
            .cells()
            .iter()
            .any(|cell| cell_text(cell.as_ref()).contains("written while open"))
    };
    assert!(in_tail(&harness));

    harness.key(KeyCode::Char('q'));
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(in_tail(&harness));
    let screen = harness.screen();
    assert!(!screen.contains("T R A N S C R I P T"), "{screen}");
    assert!(screen.contains("written while open"), "{screen}");
}

#[test]
fn raw_and_tui_save_the_interface_preferences() {
    let mut harness = Harness::owned(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    harness.type_and_submit("/raw on");
    harness.pump_until_screen_shows("Raw output on.");
    assert_eq!(harness.app.render_mode, HistoryRenderMode::Raw);
    assert!(harness.config_file().contains("rawOutput = true"));
    harness.type_and_submit("/raw");
    harness.pump_until_screen_shows("Raw output off.");
    assert_eq!(harness.app.render_mode, HistoryRenderMode::Rich);

    // `/tui` is the center's section, at its first row.
    harness.type_and_submit("/tui");
    harness.pump_until_screen_shows("› Fullscreen");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved · applies at next launch");
    let config = harness.config_file();
    assert!(config.contains("fullscreenTranscript = false"), "{config}");
    assert!(config.contains("rawOutput = false"), "{config}");
}

#[test]
fn a_resumed_session_loads_its_earlier_pages_into_the_owned_transcript() {
    let mut harness = Harness::owned(vec![
        assistant_events("First answer."),
        assistant_events("Second answer."),
        assistant_events("Third answer."),
    ]);
    for (question, answer) in [
        ("First?", "First answer."),
        ("Second?", "Second answer."),
        ("Third?", "Third answer."),
    ] {
        harness.type_and_submit(question);
        harness.pump_until_screen_shows(answer);
        harness.pump_until_turn_ends();
    }
    let finished = harness.app.bridge.session_id();
    harness.type_and_submit("/new");
    harness.pump_until("no session", |harness| {
        harness.app.bridge.session_id().is_none()
    });

    // Each page holds two items: the latest exchange, then the earlier
    // ones as the view, which has room for them, asks for them.
    harness.app.bridge.set_page_size(2);
    harness.type_and_submit("/resume");
    harness.pump_until_screen_shows("Resume a previous session");
    harness.key(KeyCode::Enter);
    harness.pump_until("every page", |harness| {
        let transcript = harness.transcript();
        harness.app.transcript_view.history == TranscriptHistoryState::Complete
            && transcript.contains("First answer.")
            && transcript.contains("Third answer.")
    });
    assert_eq!(harness.app.bridge.session_id(), finished);
    let transcript = harness.transcript();
    assert!(!transcript.contains("Earlier messages"), "{transcript}");
    let mut from = transcript.find("model:").expect(&transcript);
    for text in [
        "First?",
        "First answer.",
        "Second?",
        "Second answer.",
        "Third?",
        "Third answer.",
    ] {
        let at = transcript[from..]
            .find(text)
            .unwrap_or_else(|| panic!("{text:?} not in order:\n{transcript}"));
        from += at + text.len();
    }
}

mod agents;
mod appearance;
mod backtrack;
mod colour;
mod compaction;
mod compression;
mod context;
mod external_editor;
mod features;
mod history_search;
mod hyperlinks;
mod math;
mod mermaid;
mod paste_image;
mod plugins;
mod preferences;
mod queue;
mod rendering;
mod resize;
mod scrollback;
mod session_commands;
mod settings;
mod settings_center;
mod shell;
mod shortcuts;
mod side;
mod stability;
mod streaming;
mod subagents;
mod thinking;
mod tools;
mod workspace_commands;
