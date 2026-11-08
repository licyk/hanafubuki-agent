// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The interface's entry: a session on the started Agent, the terminal set
//! up and restored around the app loop.

use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use hanafubuki_agent::{
    AgentHandle, AgentRuntime, AgentSessionMode, AgentSessionModelSelectionRequest,
    AgentSessionRuntimeSnapshot, AgentStatusEventSink,
};

use crate::agent_bridge::{AgentBridge, StatusSink};
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
}

/// Runs the interface on the started Agent until the person quits, in a
/// new session. Standard input and output must be the terminal; the
/// process's panic hook must be installed already (the interface's own
/// restores the terminal before it). The terminal is restored on return.
pub fn run(
    events: TuiEvents,
    runtime: &AgentRuntime,
    agent: &AgentHandle,
    options: TuiOptions,
) -> Result<(), String> {
    let session = create_session(agent, options.model)?;
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
        model: session.model.clone(),
        directory: options.directory,
    });
    if let Some(prompt) = options.prompt {
        chat_widget.submit_user_message(prompt);
    }
    let bridge = AgentBridge::new(
        agent.clone(),
        runtime.slot().clone(),
        session.id,
        events.sender.clone(),
    );
    let mut app = App::new(chat_widget, bridge, events.sender.clone());
    let result = app.run(&mut tui, &events.receiver);
    drop(input);
    let left = tui.leave();
    drop(restore);
    result
        .and(left)
        .map_err(|error| format!("the terminal: {error}"))
}

/// A new session, with the model asked for.
fn create_session(
    agent: &AgentHandle,
    model: Option<String>,
) -> Result<AgentSessionRuntimeSnapshot, String> {
    let created = agent.runtime().block_on({
        let agent = agent.clone();
        async move {
            let session = agent.create_session(AgentSessionMode::Default).await?;
            match model {
                Some(model) => {
                    agent
                        .set_session_model(AgentSessionModelSelectionRequest {
                            session_id: session.id,
                            generation: session.generation,
                            revision: session.revision,
                            provider_id: session.provider_id.clone(),
                            model,
                        })
                        .await
                }
                None => Ok(session),
            }
        }
    });
    created
        .map_err(|failure| failure.to_string())?
        .map_err(|error| format!("the session was not created: {error}"))
}

/// Restores the terminal however the interface returns.
struct RestoreOnDrop;

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        let _ = tui::restore();
    }
}
