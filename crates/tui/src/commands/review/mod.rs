// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/review [instructions]`: a review of the repository's work in a child
//! session, whose findings come back into the conversation (the facade's
//! `start_review`, Codex's `/review`). Without instructions a picker
//! offers its targets (Codex's `review_popups.rs`); while the review runs
//! the status line says so and Esc, or `/review cancel`, cancels it; then
//! the review cell shows the verdict and the findings (Codex's review
//! banners and `review_format.rs`).

mod cell;
mod picker;

use hanafubuki_agent::AgentReviewTarget;

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::{ReviewCall, ReviewEnd, ReviewReply};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::history_cell;
use cell::{review_banner, ReviewCell};
use picker::ReviewPicker;

pub(crate) static REVIEW: CommandSpec = CommandSpec {
    name: "review",
    aliases: &[],
    description: "review the repository's changes and report findings",
    inline_args: true,
    // `/review cancel` runs while the review does; a review does not start
    // during a turn (dispatch says so).
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let instructions = args.trim();
    if instructions == "cancel" {
        context.agent_command(AgentCommand::Review(ReviewCall::Cancel));
        return;
    }
    if context.task_running() {
        context.add_to_history(history_cell::new_error_event(
            "'/review' is disabled while a task is in progress.".to_string(),
        ));
        return;
    }
    let call = if instructions.is_empty() {
        ReviewCall::Open
    } else {
        ReviewCall::Start(AgentReviewTarget::Custom {
            instructions: instructions.to_string(),
        })
    };
    context.agent_command(AgentCommand::Review(call));
}

pub(crate) fn on_reply(context: &mut CommandContext<'_>, reply: ReviewReply) {
    match reply {
        ReviewReply::Ready(targets) => {
            let picker = ReviewPicker::new(context.app_event_tx(), targets);
            context.show_view(Box::new(picker));
        }
        ReviewReply::Started(hint) => {
            context.set_task_status(Some(format!("Reviewing {hint}")));
            context.add_to_history(review_banner(format!(">> Code review started: {hint} <<")));
        }
        ReviewReply::Cancelling => {
            context.set_task_status(Some("Cancelling the review".to_string()))
        }
        ReviewReply::Ended(end) => {
            context.set_task_status(None);
            match end {
                ReviewEnd::Finished(output) => context.add_to_history(ReviewCell::new(*output)),
                ReviewEnd::Cancelled => {
                    context.add_to_history(review_banner("<< Code review cancelled >>".to_string()))
                }
                ReviewEnd::Failed(error) => context
                    .add_to_history(history_cell::new_error_event(format!("/review: {error}"))),
            }
        }
        ReviewReply::Failed(error) => {
            context.add_to_history(history_cell::new_error_event(format!("/review: {error}")))
        }
    }
}

/// Starts a review of `target`, from the picker.
fn start(app_event_tx: &AppEventSender, target: AgentReviewTarget) {
    app_event_tx.send(AppEvent::Op(Op::Command(AgentCommand::Review(
        ReviewCall::Start(target),
    ))));
}

#[cfg(test)]
mod tests;
