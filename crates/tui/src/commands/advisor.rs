use super::{open_settings, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::history_cell;

pub(super) static ADVISOR: CommandSpec = CommandSpec {
    name: "advisor",
    aliases: &[],
    description: "activate the configured advisor",
    inline_args: false,
    during_task: true,
    needs_session: true,
    dispatch: |ctx, _| ctx.session_call(SessionCall::AdvisorActive(true)),
};
pub(super) static OFF: CommandSpec = CommandSpec {
    name: "advisor-off",
    description: "deactivate the advisor",
    dispatch: |ctx, _| ctx.session_call(SessionCall::AdvisorActive(false)),
    ..ADVISOR
};
pub(super) static MANUAL: CommandSpec = CommandSpec {
    name: "advisor-manual",
    description: "consult the advisor once",
    inline_args: true,
    dispatch: |ctx, args| {
        ctx.add_to_history(history_cell::new_info_event(
            "Consulting the Advisor…".into(),
            None,
        ));
        ctx.session_call(SessionCall::AdvisorManual(args.to_string()));
    },
    ..ADVISOR
};
pub(super) static SETTINGS: CommandSpec = CommandSpec {
    name: "advisor-settings",
    aliases: &["advisor-models"],
    description: "configure the advisor and its models",
    needs_session: false,
    inline_args: true,
    dispatch: |ctx, args| {
        if args.trim().is_empty() {
            open_settings(ctx, "pluginOptions.advisor")
        } else {
            ctx.session_call(SessionCall::AdvisorSettings(args.into()))
        }
    },
    ..ADVISOR
};
pub(super) static STATS: CommandSpec = CommandSpec {
    name: "advisor-stats",
    description: "show retained Advisor feedback",
    needs_session: false,
    dispatch: |ctx, _| ctx.session_call(SessionCall::AdvisorStats),
    ..ADVISOR
};
