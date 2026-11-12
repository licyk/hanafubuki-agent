// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The interface's entry: the session it opens on the started Agent (one
//! reopened, or none until the first message creates it), the transcript's
//! mode for the run ([`TranscriptMode`]), and the terminal set up and
//! restored around the app loop.

use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use hanafubuki_agent::{
    AgentHandle, AgentRuntime, AgentSessionId, AgentSessionListRequest, AgentStatusEventSink,
};

use crate::agent_bridge::{with_model, AgentBridge, AgentReply, StatusSink};
use crate::app::App;
use crate::app_event::{AppEvent, AppEventSender};
use crate::appearance::Appearance;
use crate::chatwidget::{ChatWidget, ChatWidgetInit};
use crate::history_cell::HistoryRenderMode;
use crate::render::highlight::configure_theme;
use crate::transcript_mode::{is_ssh_session, TranscriptMode};
use crate::tui::{self, FrameRequester, InputReader, Tui};
use crate::tui_state::TuiState;

/// The channel the interface is told on, made before the Agent is: the
/// Agent's runtime takes its status sink ([`TuiEvents::status_sink`]).
pub struct TuiEvents {
    sender: AppEventSender,
    receiver: Receiver<AppEvent>,
}

impl TuiEvents {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender: AppEventSender::new(sender),
            receiver,
        }
    }

    /// For `AgentRuntimeHost::status`: the interface notices when the
    /// Agent stops under a running turn.
    pub fn status_sink(&self) -> Arc<dyn AgentStatusEventSink> {
        Arc::new(StatusSink(self.sender.clone()))
    }
}

/// How the interface starts.
#[derive(Clone, Debug, Default)]
pub struct TuiOptions {
    /// Submitted as the first message.
    pub prompt: Option<String>,
    /// The model of the sessions the interface creates, and of the one it
    /// resumes at the start, in place of the model policy's (`-m`).
    pub model: Option<String>,
    /// The role of the sessions the interface creates (`--role`).
    pub role: Option<String>,
    /// The working directory, as the header shows it.
    pub directory: String,
    /// A session to reopen in place of a new one.
    pub resume: Option<TuiResume>,
    /// Keeps the transcript in the terminal's scrollback, whatever
    /// `[tui]` prefers (`--no-alt-screen`).
    pub no_alt_screen: bool,
}

/// Which session `hanafubuki-agent resume` reopens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiResume {
    /// The one picked from the sessions (none until one is).
    Picker,
    /// The latest session a turn ran in; none when there is none.
    Last,
    Session(AgentSessionId),
}

/// Runs the interface on the started Agent until the person quits. Standard
/// input and output must be the terminal; the process's panic hook must be
/// installed already (the interface's own restores the terminal before it).
/// The terminal is restored on return.
pub fn run(
    events: TuiEvents,
    runtime: &Arc<AgentRuntime>,
    agent: &AgentHandle,
    options: TuiOptions,
) -> Result<(), String> {
    let bridge = AgentBridge::new(
        runtime.clone(),
        agent.clone(),
        options.model.clone(),
        options.role.clone(),
        events.sender.clone(),
    );
    let model_override = options.model.is_some();
    let opened = match resumed_session(agent, options.resume, options.model)? {
        Some(session_id) => bridge.open(session_id)?,
        None => bridge.no_session()?,
    };
    let preferences = bridge.tui_preferences();
    let theme_warning = configure_theme(
        preferences.theme.as_deref(),
        Some(runtime.storage_paths().root.join("themes")),
    );
    let (appearance, mut warnings) = Appearance::new(&preferences, options.directory.clone());
    warnings.extend(theme_warning);
    let mode = TranscriptMode::resolve(&preferences, options.no_alt_screen, is_ssh_session());
    let render_mode = if preferences.raw_output {
        HistoryRenderMode::Raw
    } else {
        HistoryRenderMode::Rich
    };
    let terminal = tui::init().map_err(|error| format!("the terminal: {error}"))?;
    let restore = RestoreOnDrop;
    let enhanced_keys_supported = tui::enhanced_keys_supported();
    let mut tui = Tui::new(terminal);
    let input = InputReader::start(events.sender.clone())
        .map_err(|error| format!("the terminal's input: {error}"))?;
    let chat_widget = ChatWidget::new(ChatWidgetInit {
        app_event_tx: events.sender.clone(),
        frame_requester: FrameRequester::new(events.sender.clone()),
        enhanced_keys_supported,
        directory: options.directory,
        owned_transcript: mode.is_owned(),
        fullscreen_preference: preferences.fullscreen_transcript,
        model_override,
        appearance,
    });
    let (tui_state, state_warning) =
        TuiState::load(&runtime.storage_paths().root, events.sender.clone());
    warnings.extend(state_warning);
    let mut app = App::new(
        chat_widget,
        bridge,
        events.sender.clone(),
        mode,
        render_mode,
        tui_state,
    );
    if mode.is_owned() {
        app.enter_owned_screen(&mut tui)
            .map_err(|error| format!("the terminal's alternate screen: {error}"))?;
    }
    if let Some(update) = app.bridge.reply(AgentReply::SessionOpened(opened)) {
        app.show_update(update);
    }
    warnings.extend(app.apply_preferences(preferences));
    let config_warnings = app.bridge.config_warnings();
    app.chat_widget
        .add_startup_warnings(warnings, config_warnings);
    if let Some(prompt) = options.prompt {
        app.chat_widget.submit_user_message(prompt);
    }
    if options.resume == Some(TuiResume::Picker) {
        app.open_resume_picker(&mut tui, true);
    }
    let result = app.run(&mut tui, &events.receiver);
    app.save_state_on_exit();
    app.restore_terminal_title(&mut tui);
    drop(input);
    if mode.is_owned() {
        let _ = tui.leave_alt_screen();
    }
    let left = tui.leave();
    drop(restore);
    result
        .and(left)
        .map_err(|error| format!("the terminal: {error}"))
}

/// The session to resume at the start, switched to the model asked for;
/// `None` for none.
fn resumed_session(
    agent: &AgentHandle,
    resume: Option<TuiResume>,
    model: Option<String>,
) -> Result<Option<AgentSessionId>, String> {
    let session_id = match resume {
        Some(TuiResume::Session(session_id)) => session_id,
        Some(TuiResume::Last) => match latest_session(agent)? {
            Some(session_id) => session_id,
            None => return Ok(None),
        },
        Some(TuiResume::Picker) | None => return Ok(None),
    };
    let session = agent
        .open_session(session_id, 1)
        .map_err(|error| format!("the session was not opened: {error}"))?
        .runtime;
    if let Some(model) = model {
        let call = agent.clone();
        agent
            .runtime()
            .block_on(async move { with_model(&call, session, model).await })
            .map_err(|failure| failure.to_string())??;
    }
    Ok(Some(session_id))
}

/// The latest session a turn ran in.
fn latest_session(agent: &AgentHandle) -> Result<Option<AgentSessionId>, String> {
    let mut request = AgentSessionListRequest::default();
    loop {
        let page = agent
            .list_sessions_page(request.clone())
            .map_err(|error| format!("the sessions were not listed: {error}"))?;
        if let Some(session) = page
            .sessions
            .iter()
            .find(|session| session.latest_turn_phase.is_some())
        {
            return Ok(Some(session.session_id));
        }
        match page.next_cursor {
            Some(cursor) => request.cursor = Some(cursor),
            None => return Ok(None),
        }
    }
}

/// Restores the terminal however the interface returns.
struct RestoreOnDrop;

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        let _ = tui::restore();
    }
}
