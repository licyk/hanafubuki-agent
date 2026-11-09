// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The interface's entry: a session on the started Agent (a new one, or
//! one reopened), the terminal set up and restored around the app loop.

use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use hanafubuki_agent::{
    AgentHandle, AgentRuntime, AgentSessionId, AgentSessionListRequest, AgentSessionMode,
    AgentSessionModelSelectionRequest, AgentStatusEventSink,
};

use crate::agent_bridge::{AgentBridge, StatusSink, TurnUpdate};
use crate::app::App;
use crate::app_event::{AppEvent, AppEventSender};
use crate::chatwidget::{ChatWidget, ChatWidgetInit};
use crate::tui::{self, FrameRequester, InputReader, Tui};

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
    /// The session's model, in place of the model policy's (`-m`).
    pub model: Option<String>,
    /// The working directory, as the header shows it.
    pub directory: String,
    /// A session to reopen in place of a new one.
    pub resume: Option<TuiResume>,
}

/// Which session `hanafubuki-agent resume` reopens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiResume {
    /// The one picked from the sessions (a new session until one is).
    Picker,
    /// The latest session a turn ran in; a new session when there is none.
    Last,
    Session(AgentSessionId),
}

/// Runs the interface on the started Agent until the person quits. Standard
/// input and output must be the terminal; the process's panic hook must be
/// installed already (the interface's own restores the terminal before it).
/// The terminal is restored on return.
pub fn run(
    events: TuiEvents,
    runtime: &AgentRuntime,
    agent: &AgentHandle,
    options: TuiOptions,
) -> Result<(), String> {
    let session_id = open_session(agent, options.resume, options.model)?;
    let bridge = AgentBridge::new(
        agent.clone(),
        runtime.slot().clone(),
        session_id,
        events.sender.clone(),
    );
    let opened = bridge.open(session_id)?;
    let terminal = tui::init().map_err(|error| format!("the terminal: {error}"))?;
    let restore = RestoreOnDrop;
    let enhanced_keys_supported = tui::enhanced_keys_supported();
    let mut tui = Tui::new(terminal);
    let input = InputReader::start(events.sender.clone())
        .map_err(|error| format!("the terminal's input: {error}"))?;
    let mut chat_widget = ChatWidget::new(ChatWidgetInit {
        app_event_tx: events.sender.clone(),
        frame_requester: FrameRequester::new(events.sender.clone()),
        enhanced_keys_supported,
        directory: options.directory,
    });
    chat_widget.handle_turn_update(TurnUpdate::SessionOpened(opened));
    if let Some(prompt) = options.prompt {
        chat_widget.submit_user_message(prompt);
    }
    let mut app = App::new(chat_widget, bridge, events.sender.clone());
    if options.resume == Some(TuiResume::Picker) {
        app.open_resume_picker(&mut tui, true);
    }
    let result = app.run(&mut tui, &events.receiver);
    drop(input);
    let left = tui.leave();
    drop(restore);
    result
        .and(left)
        .map_err(|error| format!("the terminal: {error}"))
}

/// The session the interface starts on, with the model asked for: the one
/// to resume, or a new one.
fn open_session(
    agent: &AgentHandle,
    resume: Option<TuiResume>,
    model: Option<String>,
) -> Result<AgentSessionId, String> {
    let resumed = match resume {
        Some(TuiResume::Session(session_id)) => Some(session_id),
        Some(TuiResume::Last) => latest_session(agent)?,
        Some(TuiResume::Picker) | None => None,
    };
    let opened = agent.runtime().block_on({
        let agent = agent.clone();
        async move {
            let session = match resumed {
                Some(session_id) => agent
                    .open_session(session_id, 1)
                    .map(|opened| opened.runtime)
                    .map_err(|error| format!("the session was not opened: {error}"))?,
                None => agent
                    .create_session(AgentSessionMode::Default)
                    .await
                    .map_err(|error| format!("the session was not created: {error}"))?,
            };
            match model {
                Some(model) => agent
                    .set_session_model(AgentSessionModelSelectionRequest {
                        session_id: session.id,
                        generation: session.generation,
                        revision: session.revision,
                        provider_id: session.provider_id.clone(),
                        model,
                    })
                    .await
                    .map_err(|error| format!("the model was not set: {error}")),
                None => Ok(session),
            }
        }
    });
    Ok(opened.map_err(|failure| failure.to_string())??.id)
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
