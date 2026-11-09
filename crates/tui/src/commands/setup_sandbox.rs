// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/setup-default-sandbox` (Windows only, as in Codex): Codex runs the
//! elevated sandbox's setup here (sandbox users an administrator creates,
//! the network blocked by firewall rules) and then writes
//! `[windows] sandbox = "elevated"`. That sandbox is not ported yet, so the
//! command says so and how to use the unelevated one, which needs no setup.

use super::{CommandContext, CommandSpec};
use crate::history_cell;

pub(super) static SETUP_DEFAULT_SANDBOX: CommandSpec = CommandSpec {
    name: "setup-default-sandbox",
    aliases: &[],
    description: "set up the elevated Windows sandbox",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| explain(context),
};

const NOT_YET: &str = "The elevated Windows sandbox (sandbox users an administrator \
     creates, the network blocked by firewall rules) is not available yet, so there \
     is nothing to set up.";
const UNELEVATED: &str = "The unelevated sandbox needs no setup: with \
     `[windows] sandbox = \"unelevated\"` and `sandboxMode = \"workspace-write\"` (or \
     \"read-only\") in config.toml, a restricted token confines what commands write; \
     it does not block the network.";

fn explain(context: &mut CommandContext<'_>) {
    if !cfg!(windows) {
        return context.add_to_history(history_cell::new_info_event(
            "/setup-default-sandbox is for Windows; the sandbox here needs no setup.".to_string(),
            None,
        ));
    }
    context.add_to_history(history_cell::new_info_event(
        NOT_YET.to_string(),
        Some(UNELEVATED.to_string()),
    ));
}
