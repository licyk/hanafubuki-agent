// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Explicit elevated sandbox setup. UAC and provisioning run off the UI loop;
//! only successful setup persists the sandbox configuration.

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::history_cell;

pub(super) static SETUP_DEFAULT_SANDBOX: CommandSpec = CommandSpec {
    name: "setup-default-sandbox",
    aliases: &[],
    description: "set up the elevated Windows sandbox",
    inline_args: false,
    during_task: false,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    if !cfg!(windows) {
        return context.add_to_history(history_cell::new_info_event(
            "/setup-default-sandbox is for Windows; the sandbox here needs no setup.".into(),
            None,
        ));
    }
    context.add_to_history(history_cell::new_info_event(
        "Setting up the Windows sandbox; approve the Windows administrator prompt to continue."
            .into(),
        Some("The configuration is saved only after setup succeeds.".into()),
    ));
    context.session_call(SessionCall::SetupWindowsSandbox);
}

pub(super) fn on_ready(context: &mut CommandContext<'_>) {
    context.add_to_history(history_cell::new_info_event(
        "Windows sandbox ready: elevated sandbox and workspace-write saved.".into(),
        Some("Use /permissions to choose a sandboxed preset for this session; new sessions use the saved configuration.".into()),
    ));
}
