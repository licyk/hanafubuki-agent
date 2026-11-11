// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! `/role [NAME]`: with a name, the session takes that role
//! (site/content/docs/roles.en.mdx); without one, the list of the usable
//! roles, the session's marked.

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::RoleChoices;
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};

pub(crate) static ROLE: CommandSpec = CommandSpec {
    name: "role",
    aliases: &[],
    description: "choose the role of this session",
    inline_args: true,
    during_task: false,
    needs_session: true,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let name = args.trim();
    context.agent_command(if name.is_empty() {
        AgentCommand::ListRoles
    } else {
        AgentCommand::SetRole(name.to_string())
    });
}

/// The picker over the usable roles, as `/model` is over the models.
pub(crate) fn show_role_choices(context: &mut CommandContext<'_>, choices: RoleChoices) {
    let current = choices.current.as_deref().unwrap_or("default");
    let items = choices
        .roles
        .into_iter()
        .map(|(name, description)| SelectionItem {
            is_current: name == current,
            description: Some(description.lines().next().unwrap_or_default().to_string()),
            actions: vec![Box::new({
                let name = name.clone();
                move |tx| {
                    tx.send(AppEvent::Op(Op::Command(AgentCommand::SetRole(
                        name.clone(),
                    ))))
                }
            })],
            name,
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();
    context.show_selection_view(SelectionViewParams {
        title: Some("Select Role".to_string()),
        subtitle: Some("Switch the role of this session.".to_string()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search roles".to_string()),
        ..Default::default()
    });
}
