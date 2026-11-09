// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! `/mode`: the list of the session's modes, the current one marked.

use hanafubuki_agent::AgentSessionMode;

use super::{CommandContext, CommandSpec};
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};

pub(super) static MODE: CommandSpec = CommandSpec {
    name: "mode",
    aliases: &[],
    description: "switch between default and plan mode",
    inline_args: false,
    during_task: false,
    needs_session: true,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    let current = context.mode();
    let items = [AgentSessionMode::Default, AgentSessionMode::Plan]
        .into_iter()
        .map(|mode| SelectionItem {
            name: mode_name(mode).to_string(),
            description: Some(
                match mode {
                    AgentSessionMode::Default => "the Agent works on the task",
                    AgentSessionMode::Plan => {
                        "read-only exploration that ends with a proposed plan"
                    }
                }
                .to_string(),
            ),
            is_current: mode == current,
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::Op(Op::Command(AgentCommand::SetMode(mode))))
            })],
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();
    context.show_selection_view(SelectionViewParams {
        title: Some("Select Mode".to_string()),
        subtitle: Some("Switch the mode of this session.".to_string()),
        items,
        ..Default::default()
    });
}

pub(crate) fn mode_name(mode: AgentSessionMode) -> &'static str {
    match mode {
        AgentSessionMode::Default => "default",
        AgentSessionMode::Plan => "plan",
    }
}
