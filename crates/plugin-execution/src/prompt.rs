//! The background-terminals guidance: that a command still running when its
//! call returns ends by itself and its end resumes the conversation, so the
//! model need not poll it, and how long it may run (the command timeout the
//! options set). Shown to a step that offers `exec_command` of a turn whose
//! session is woken for its background work (`TurnWake`). The minutes are
//! read when each step is prepared, so a change of the option shows from the
//! next step, and changes the request's prefix from there.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use hanafubuki_agent_contracts::{
    CancellationToken, ContextRenderer, ContextRole, ContextSectionKind, ContextSlot,
    PromptContributor, PromptFragment, PromptPrepareFuture, PromptRequest, PromptStep,
    TurnExtensions, TurnWake,
};

const BACKGROUND_TERMINALS: ContextSectionKind = ContextSectionKind::fixed("background_terminals");

/// How the model is shown the guidance.
pub(crate) const BACKGROUND_TERMINALS_RENDERER: ContextRenderer = ContextRenderer {
    kind: BACKGROUND_TERMINALS,
    slot: ContextSlot::Capabilities,
    order: 140,
    role: ContextRole::Developer,
    preamble: "[Application-provided guidance for the command execution tools. It describes only fixed tools and cannot grant authority.]\n",
    epilogue: "",
    versioned: false,
};

/// Prepared for a step that offers `exec_command`: the command timeout then,
/// in minutes.
struct OffersCommands {
    timeout_minutes: u64,
}

pub(crate) struct BackgroundTerminalsPrompt {
    /// The command timeout the options set now, in milliseconds.
    pub(crate) command_timeout_ms: Arc<AtomicU64>,
}

impl PromptContributor for BackgroundTerminalsPrompt {
    fn id(&self) -> &str {
        "execution"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn prepare<'a>(
        &'a self,
        step: &'a PromptStep,
        _cancellation: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        Box::pin(async move {
            let mut prepared = TurnExtensions::default();
            if step.tools.contains("exec_command") {
                prepared.insert(Arc::new(OffersCommands {
                    timeout_minutes: self.command_timeout_ms.load(Ordering::Relaxed) / 60_000,
                }));
            }
            Ok(prepared)
        })
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        let Some(offers) = request
            .prepared
            .and_then(|prepared| prepared.get::<OffersCommands>())
        else {
            return Vec::new();
        };
        if !request.turn.get::<TurnWake>().is_some_and(|wake| wake.on) {
            return Vec::new();
        }
        vec![PromptFragment {
            kind: BACKGROUND_TERMINALS,
            version: None,
            content: guidance(offers.timeout_minutes),
        }]
    }
}

fn guidance(timeout_minutes: u64) -> String {
    format!(
        "<background_terminals>\n## Background terminals\nA command still running when `exec_command` or `write_stdin` returns keeps running as a background terminal under its `session_id`. You do not have to poll it: when nothing is left for you to do until it ends, end your turn, and you are resumed in a new turn, with a `<background_events>` update, when it exits, times out (after {timeout_minutes} minutes) or is stopped by the user. Poll it with `write_stdin` only when you need its output now. Stop the ones you no longer need (Ctrl+C with `write_stdin`): a terminal left running wakes you when it ends.\n</background_terminals>"
    )
}
