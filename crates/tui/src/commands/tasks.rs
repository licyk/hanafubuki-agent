use super::{CommandContext, CommandSpec};
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;
use crate::tasks::TasksEvent;

pub(super) static TASKS: CommandSpec = CommandSpec {
    name: "tasks",
    aliases: &[],
    description: "manage background agents and terminals",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context: &mut CommandContext<'_>, _| {
        context.send(AppEvent::Workspace(WorkspaceEvent::Tasks(Box::new(
            TasksEvent::Open,
        ))));
    },
};
