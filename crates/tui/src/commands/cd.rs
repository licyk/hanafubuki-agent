// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/cd [path]` (the home directory without one): the working directory
//! moves, a relative path read against the current one, and the session
//! goes on there (Codex's `/cd`). The Agent refuses while a turn, a
//! compaction or a background terminal runs. The project configuration
//! stays the one read at start.

use std::path::PathBuf;

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::history_cell;

pub(super) static CD: CommandSpec = CommandSpec {
    name: "cd",
    aliases: &[],
    description: "change the working directory",
    inline_args: true,
    during_task: false,
    needs_session: false,
    dispatch,
};

/// What `/cd` adds once the directory moved.
pub(super) const PROJECT_NOTE: &str = "The project configuration (its trust level and its \
     config.toml) stays the one read at start until you restart.";

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    match expand_home(args.trim()) {
        Some(path) => context.session_call(SessionCall::ChangeDirectory(path)),
        None => context.add_to_history(history_cell::new_error_event(
            "/cd: there is no home directory to change to".to_string(),
        )),
    }
}

/// `path`, with `~` (or nothing) for the home directory; `None` when `~`
/// is asked for and there is no home directory.
pub(super) fn expand_home(path: &str) -> Option<PathBuf> {
    let rest = match path {
        "" | "~" => "",
        _ => match path
            .strip_prefix("~/")
            .or_else(|| path.strip_prefix("~\\").filter(|_| cfg!(windows)))
        {
            Some(rest) => rest,
            None => return Some(PathBuf::from(path)),
        },
    };
    let home = std::env::home_dir()?;
    Some(if rest.is_empty() {
        home
    } else {
        home.join(rest)
    })
}

pub(super) fn on_reply(context: &mut CommandContext<'_>, roots: Vec<PathBuf>) {
    let mut roots = roots.into_iter();
    if let Some(first) = roots.next() {
        context.add_to_history(history_cell::new_info_event(
            format!("Working directory changed to {}", first.display()),
            Some(PROJECT_NOTE.to_string()),
        ));
    }
    for root in roots {
        context.add_to_history(history_cell::new_info_event(
            format!("Also in the workspace: {}", root.display()),
            None,
        ));
    }
}
