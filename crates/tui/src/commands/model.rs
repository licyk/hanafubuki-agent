// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! `/model`: the bridge lists the Providers' models, and the picker over
//! them (Codex's model popup) sets the session's.

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::ModelChoices;
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell;
use crate::model_facts::entry_badges;

pub(crate) static MODEL: CommandSpec = CommandSpec {
    name: "model",
    aliases: &[],
    description: "choose what model to use",
    inline_args: true,
    during_task: false,
    needs_session: true,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    match args.trim() {
        "" => context.agent_command(AgentCommand::ListModels),
        "forget" => context.agent_command(AgentCommand::ForgetLearnedModelFacts),
        _ => context.add_to_history(history_cell::new_error_event(
            "Usage: /model [forget]".to_string(),
        )),
    }
}

/// The picker over the Providers' catalogs: every model each lists, and
/// the session's own when no catalog has it.
pub(crate) fn show_model_choices(context: &mut CommandContext<'_>, choices: ModelChoices) {
    let mut items = Vec::new();
    let listed = choices.catalogs.iter().any(|catalog| {
        catalog.provider_id == choices.provider_id
            && catalog
                .models
                .iter()
                .any(|model| model.entry.id == choices.model)
    });
    if !listed && !choices.model.is_empty() {
        items.push(model_item(
            &choices.provider_id,
            &choices.model,
            choices.provider_id.clone(),
            true,
        ));
    }
    for catalog in &choices.catalogs {
        if let Some(error) = &catalog.error {
            items.push(SelectionItem {
                name: catalog.display_name.clone(),
                description: Some(format!("models not listed: {error}")),
                is_disabled: true,
                ..Default::default()
            });
            continue;
        }
        for model in &catalog.models {
            let current =
                catalog.provider_id == choices.provider_id && model.entry.id == choices.model;
            let description = match entry_badges(&model.metadata) {
                Some(badges) => format!("{} · {badges}", catalog.display_name),
                None => catalog.display_name.clone(),
            };
            items.push(model_item(
                &catalog.provider_id,
                &model.entry.id,
                description,
                current,
            ));
        }
    }
    context.show_selection_view(SelectionViewParams {
        title: Some("Select Model".to_string()),
        subtitle: Some("Switch the model of this session.".to_string()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search models".to_string()),
        ..Default::default()
    });
}

fn model_item(provider_id: &str, model: &str, provider: String, current: bool) -> SelectionItem {
    let provider_id = provider_id.to_string();
    let model = model.to_string();
    SelectionItem {
        name: model.clone(),
        description: Some(provider),
        is_current: current,
        search_value: Some(model.clone()),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::Op(Op::Command(AgentCommand::SetModel {
                provider_id: provider_id.clone(),
                model: model.clone(),
            })))
        })],
        dismiss_on_select: true,
        ..Default::default()
    }
}
