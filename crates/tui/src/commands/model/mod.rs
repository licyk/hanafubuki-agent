// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! `/model`: a list of the Providers' models (Codex's model popup, `view`),
//! open at once and filled as each Provider's listing answers, that sets
//! the session's; the same list adds a model's own settings, from the
//! settings center's Model overrides ([`open`]).

mod entry;
pub(super) mod footer;
mod purpose;
mod view;

use std::sync::atomic::{AtomicU64, Ordering};

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::model_list::ModelListUpdate;
use crate::app_event::AgentCommand;
use crate::bottom_pane::LoadingLine;
use crate::history_cell;
use crate::settings_center::{Attach, PageOverride};
use crate::tui::FrameRequester;
pub(crate) use purpose::Purpose;
use view::ModelListView;

pub(crate) static MODEL: CommandSpec = CommandSpec {
    name: "model",
    aliases: &[],
    description: "choose what model to use",
    inline_args: true,
    // A running turn takes the model at its next step.
    during_task: true,
    needs_session: true,
    dispatch,
};

/// The Model overrides collection's `a`: this same list, over every
/// Provider, adding the pair chosen as a model's settings of its own.
pub(crate) static OVERRIDES_PAGE: PageOverride = PageOverride {
    id: "provider-model-overrides",
    attach: Attach::Add {
        collection: "providerModels",
    },
    command: &MODEL,
    open: open_overrides,
};

/// The number of the latest list opened: a list hears only its own
/// listings' answers.
static LISTINGS: AtomicU64 = AtomicU64::new(0);

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    match args.trim() {
        "" => open(context, Purpose::Session),
        "forget" => context.agent_command(AgentCommand::ForgetLearnedModelFacts),
        _ => context.add_to_history(history_cell::new_error_event(
            "Usage: /model [forget]".to_string(),
        )),
    }
}

/// Opens the list as `purpose`'s: `/model`'s, or the settings center's
/// Model overrides.
pub(crate) fn open(context: &mut CommandContext<'_>, purpose: Purpose) {
    let listing = LISTINGS.fetch_add(1, Ordering::Relaxed) + 1;
    let tx = context.app_event_tx();
    let loading = LoadingLine::new(FrameRequester::new(tx.clone()), true);
    context.show_view(Box::new(ModelListView::new(listing, purpose, tx, loading)));
    context.agent_command(AgentCommand::ListModels {
        listing,
        provider_ids: None,
    });
}

fn open_overrides(context: &mut CommandContext<'_>, _record: Option<&str>) {
    open(
        context,
        Purpose::AddRecord {
            collection: "providerModels".to_string(),
        },
    );
}

/// What a listing heard, for the list open; with none open (closed, or
/// another command's view in its place) it is dropped.
pub(crate) fn on_model_list(context: &mut CommandContext<'_>, update: ModelListUpdate) {
    let _ = context.offer_reply(CommandReply::ModelList(update));
}

#[cfg(test)]
mod tests;
