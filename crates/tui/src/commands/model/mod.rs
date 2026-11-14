// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! `/model`: a list of the Providers' models (Codex's model popup, `view`),
//! open at once and filled as each Provider's listing answers, that sets
//! the session's.

pub(super) mod footer;
mod view;

use std::sync::atomic::{AtomicU64, Ordering};

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::model_list::ModelListUpdate;
use crate::app_event::AgentCommand;
use crate::bottom_pane::LoadingLine;
use crate::history_cell;
use crate::tui::FrameRequester;
use view::ModelListView;

pub(crate) static MODEL: CommandSpec = CommandSpec {
    name: "model",
    aliases: &[],
    description: "choose what model to use",
    inline_args: true,
    during_task: false,
    needs_session: true,
    dispatch,
};

/// The number of the latest list opened: a list hears only its own
/// listings' answers.
static LISTINGS: AtomicU64 = AtomicU64::new(0);

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    match args.trim() {
        "" => {
            let listing = LISTINGS.fetch_add(1, Ordering::Relaxed) + 1;
            let tx = context.app_event_tx();
            let loading = LoadingLine::new(FrameRequester::new(tx.clone()), true);
            context.show_view(Box::new(ModelListView::new(listing, tx, loading)));
            context.agent_command(AgentCommand::ListModels {
                listing,
                provider_ids: None,
            });
        }
        "forget" => context.agent_command(AgentCommand::ForgetLearnedModelFacts),
        _ => context.add_to_history(history_cell::new_error_event(
            "Usage: /model [forget]".to_string(),
        )),
    }
}

/// What a listing heard, for the list open; with none open (closed, or
/// another command's view in its place) it is dropped.
pub(crate) fn on_model_list(context: &mut CommandContext<'_>, update: ModelListUpdate) {
    let _ = context.offer_reply(CommandReply::ModelList(update));
}

#[cfg(test)]
mod tests;
