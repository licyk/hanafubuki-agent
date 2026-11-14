// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/providers`: the settings center's Providers, where each is added,
//! edited, enabled, deleted, tested (List models) or made the default;
//! `/providers policy` opens the model policy. A Provider's record has the
//! model page (Codex's model choice): its models, one of which, or an id
//! typed in, becomes the model new sessions run on it. The center is the
//! first-run setup too: with no default Provider or no model it opens at
//! the Providers, and again when a message is sent before they are set.

mod models;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::ProviderModels;
use crate::app_event::AgentCommand;
use crate::bottom_pane::LoadingLine;
use crate::history_cell;
use crate::settings_center::{Attach, PageOverride};
use crate::tui::FrameRequester;
use models::ModelPicker;

pub(crate) static PROVIDERS: CommandSpec = CommandSpec {
    name: "providers",
    aliases: &[],
    description: "add, edit and test model Providers, and choose the default model",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

/// The settings center's page of a Provider's models.
pub(crate) static MODEL_PAGE: PageOverride = PageOverride {
    id: "provider-model",
    attach: Attach::Record {
        collection: "providers",
        label: "Choose the model",
    },
    command: &PROVIDERS,
    open: |context, record| {
        let Some(provider_id) = record else {
            return;
        };
        let tx = context.app_event_tx();
        let loading = LoadingLine::new(FrameRequester::new(tx.clone()), true);
        context.show_view(Box::new(ModelPicker::new(
            provider_id.to_string(),
            tx,
            loading,
        )));
        context.agent_command(AgentCommand::ProviderModels(provider_id.to_string()));
    },
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    match args.trim() {
        "" => super::open_settings(context, "providers"),
        "policy" => super::open_settings(context, "modelPolicy"),
        _ => context.add_to_history(history_cell::new_error_event(
            "Usage: /providers [policy]".to_string(),
        )),
    }
}

/// The models listed go to the open model page; with none open they are
/// dropped.
pub(crate) fn on_models(context: &mut CommandContext<'_>, models: ProviderModels) {
    let _ = context.offer_reply(CommandReply::ProviderModels(models));
}

#[cfg(test)]
mod tests;
