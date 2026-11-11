//! `exec`: one turn without the terminal interface. The turn's events go to
//! standard output as text or as JSON lines; approvals and questions are
//! answered by the `--approve` policy and, on a terminal, by the person at
//! it; the exit code says how the turn ended (site/content/docs/exec.en.mdx, "Exit codes").

mod answers;
mod output;

use std::io::{IsTerminal, Read};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use hanafubuki_agent::{
    AgentAvailability, AgentEvent, AgentEventKind, AgentEventSink, AgentHandle, AgentRuntimeHost,
    AgentSessionMode, AgentSessionModelSelectionRequest, AgentStatusEventSink, AgentTurnAccepted,
    AgentTurnRequest, AgentTurnTerminalReason,
};

use self::answers::{Answers, ApprovePolicy};
pub(crate) use self::output::thought_for;
use self::output::{Human, JsonLines, Output};
use crate::agent::{block_on, Standalone};
use crate::cli::{ExecArgs, GlobalArgs};
use crate::exit::{Failure, CANCELLED, FAILURE, SUCCESS, UNANSWERED};

pub(crate) fn run(global: &GlobalArgs, args: ExecArgs) -> Result<u8, Failure> {
    let prompt = prompt(args.prompt.as_deref())?;
    let policy = ApprovePolicy::parse(&args.approve).map_err(Failure::usage)?;
    let (sender, receiver) = channel();
    let host = AgentRuntimeHost {
        status: Arc::new(Channel(sender.clone())),
        ..AgentRuntimeHost::default()
    };
    let standalone = Standalone::open(global, host)?;
    let mut output: Box<dyn Output> = if args.json {
        Box::new(JsonLines::default())
    } else {
        Box::new(Human::default())
    };
    let result = standalone.start().and_then(|agent| {
        Turn {
            standalone: &standalone,
            agent: &agent,
            answers: Answers::new(policy),
            output: output.as_mut(),
            receiver,
        }
        .run(global.model.clone(), global.role.clone(), prompt, sender)
    });
    standalone.shutdown();
    let code = result?;
    if let Some(path) = &args.output_last_message {
        std::fs::write(path, output.final_message().unwrap_or_default())
            .map_err(|error| Failure::new(format!("write {}: {error}", path.display())))?;
    }
    Ok(code)
}

/// The prompt from the command line, or from standard input for `-` or
/// none when standard input is not a terminal.
fn prompt(argument: Option<&str>) -> Result<String, Failure> {
    match argument {
        Some("-") => {}
        Some(prompt) => return Ok(prompt.to_string()),
        None if std::io::stdin().is_terminal() => {
            return Err(Failure::usage(
                "exec needs a prompt: an argument, or `-` to read it from standard input",
            ))
        }
        None => {}
    }
    let mut prompt = String::new();
    std::io::stdin()
        .read_to_string(&mut prompt)
        .map_err(|error| Failure::new(format!("read the prompt: {error}")))?;
    if prompt.trim().is_empty() {
        return Err(Failure::usage("the prompt on standard input is empty"));
    }
    Ok(prompt)
}

enum Message {
    Event(Box<AgentEvent>),
    StatusChanged,
    Interrupted,
}

/// Forwards what the Agent tells to the turn's loop: `emit` must not block.
struct Channel(Sender<Message>);

impl AgentEventSink for Channel {
    fn emit(&self, event: AgentEvent) {
        let _ = self.0.send(Message::Event(Box::new(event)));
    }
}

impl AgentStatusEventSink for Channel {
    fn status_changed(&self) {
        let _ = self.0.send(Message::StatusChanged);
    }
}

/// The turn's loop, on the calling thread: it prints each event and answers
/// what the turn asks, until the turn ends.
struct Turn<'a> {
    standalone: &'a Standalone,
    agent: &'a AgentHandle,
    answers: Answers,
    output: &'a mut dyn Output,
    receiver: Receiver<Message>,
}

impl Turn<'_> {
    fn run(
        mut self,
        model: Option<String>,
        role: Option<String>,
        prompt: String,
        sender: Sender<Message>,
    ) -> Result<u8, Failure> {
        let turn = self.submit(model, role, prompt, sender.clone())?;
        watch_interrupts(self.agent, sender);
        let mut unanswered = false;
        loop {
            let event = match self.receiver.recv() {
                Ok(Message::Event(event)) => *event,
                Ok(Message::StatusChanged) => {
                    self.check_running()?;
                    continue;
                }
                Ok(Message::Interrupted) => {
                    self.output.note("interrupted; cancelling the turn");
                    self.cancel(turn);
                    continue;
                }
                Err(_) => return Err(Failure::new("the Agent stopped telling about the turn")),
            };
            self.output.event(&event);
            match &event.kind {
                AgentEventKind::ToolStarted { call_id, name, .. } => {
                    self.answers.tool_started(call_id, name)
                }
                AgentEventKind::ApprovalRequested { request } => {
                    let decision = self.answers.approval(request, self.output);
                    if let Err(error) = self.agent.resolve_approval(
                        turn.session_id,
                        turn.turn_id,
                        turn.generation,
                        request.request_id,
                        decision,
                    ) {
                        self.output
                            .note(&format!("the approval was not answered: {error}"));
                    }
                }
                AgentEventKind::UserInputRequested { request } => {
                    match self.answers.questions(request, self.output) {
                        Some(answers) => {
                            if let Err(error) = self.agent.resolve_user_input(
                                turn.session_id,
                                turn.turn_id,
                                turn.generation,
                                request.request_id,
                                answers,
                            ) {
                                self.output
                                    .note(&format!("the question was not answered: {error}"));
                            }
                        }
                        None => {
                            unanswered = true;
                            self.output.note(
                                "the model asked a question that no one at a terminal can \
                                 answer; cancelling the turn",
                            );
                            self.cancel(turn);
                        }
                    }
                }
                AgentEventKind::TurnCompleted { .. } => return Ok(SUCCESS),
                AgentEventKind::TurnCancelled { .. } if unanswered => return Ok(UNANSWERED),
                AgentEventKind::TurnCancelled {
                    reason: AgentTurnTerminalReason::UserInterrupted,
                } => return Ok(CANCELLED),
                AgentEventKind::TurnCancelled { .. } | AgentEventKind::TurnFailed { .. } => {
                    return Ok(FAILURE)
                }
                _ => {}
            }
        }
    }

    fn submit(
        &mut self,
        model: Option<String>,
        role: Option<String>,
        prompt: String,
        sender: Sender<Message>,
    ) -> Result<AgentTurnAccepted, Failure> {
        let session = block_on(self.agent, |agent| async move {
            let session = agent
                .create_session(AgentSessionMode::Default, role)
                .await?;
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
        })?;
        self.output.session(session.id);
        block_on(self.agent, move |agent| async move {
            let request = AgentTurnRequest {
                text: prompt,
                images: Vec::new(),
                skills: Vec::new(),
                host: Default::default(),
            };
            agent
                .submit_turn_with_sink(session.id, request, Arc::new(Channel(sender)))
                .await
        })
    }

    fn cancel(&mut self, turn: AgentTurnAccepted) {
        let cancelled = block_on(self.agent, move |agent| async move {
            agent
                .cancel_turn(turn.session_id, turn.turn_id, turn.generation)
                .await
        });
        if let Err(failure) = cancelled {
            self.output
                .note(&format!("the turn was not cancelled: {}", failure.message));
        }
    }

    /// Fails the turn when the Agent it runs in no longer runs (a kernel
    /// fault stopped it, and a restart would not have it).
    fn check_running(&self) -> Result<(), Failure> {
        let status = self.standalone.runtime.slot().status();
        match status.availability {
            AgentAvailability::Running if status.generation == self.agent.generation() => Ok(()),
            AgentAvailability::Faulted { reason, .. } => Err(Failure::new(format!(
                "the Agent stopped after an internal error, and the turn with it: {reason}"
            ))),
            _ => Err(Failure::new("the Agent stopped during the turn")),
        }
    }
}

/// Ctrl-C cancels the turn; a second one ends the process at once.
fn watch_interrupts(agent: &AgentHandle, sender: Sender<Message>) {
    agent.runtime().handle().spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = sender.send(Message::Interrupted);
            if tokio::signal::ctrl_c().await.is_ok() {
                std::process::exit(i32::from(CANCELLED));
            }
        }
    });
}
