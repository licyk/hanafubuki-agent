//! What keeps a review session read-only, in place of the read-only sandbox
//! Codex reviews in: an approval rule that denies every action of a session
//! of origin `Review` that needs approval, except a plain read-only `git`
//! command. Tools whose effect is read-only need no approval and run as
//! anywhere else. A review turn runs in `FullAccess`, so what the rule lets
//! through runs without asking (Codex's "approval never"); a rule that
//! fails denies.

use hanafubuki_agent_approval::{ApprovalAction, ApprovalActionKind, PermissionActionDetails};
use hanafubuki_agent_contracts::{AgentSessionOrigin, ApprovalRule, ApprovalRuleVerdict};
use hanafubuki_agent_execution::POWERSHELL_UTF8_OUTPUT_PREFIX;

/// The `git` commands that only read. Each is run with `--no-pager`, so no
/// pager waits on a terminal for input a later `write_stdin` could give it.
const READ_ONLY_GIT_COMMANDS: [&str; 8] = [
    "status",
    "diff",
    "show",
    "log",
    "blame",
    "rev-parse",
    "merge-base",
    "ls-files",
];

/// Options of those commands that write a file or run another program.
const WRITING_OPTIONS: [&str; 2] = ["--output", "--ext-diff"];

pub(super) const READ_ONLY_REASON: &str = "This review is read-only: it runs only `git --no-pager <command>` for one of status, diff, show, log, blame, rev-parse, merge-base and ls-files, with plain arguments (no quotes, variables, redirections, pipes or other commands). Read files with the file tools, and change nothing.";

pub(super) struct ReviewIsReadOnly;

impl ApprovalRule for ReviewIsReadOnly {
    fn id(&self) -> &str {
        "review.read-only"
    }

    fn evaluate(
        &self,
        origin: Option<AgentSessionOrigin>,
        action: &ApprovalAction,
    ) -> Option<ApprovalRuleVerdict> {
        if origin != Some(AgentSessionOrigin::Review) || reads_only(action) {
            return None;
        }
        Some(ApprovalRuleVerdict::Deny {
            reason: READ_ONLY_REASON.to_string(),
        })
    }
}

/// Whether `action` runs a read-only `git` command through a POSIX shell or
/// PowerShell. `cmd` is never one: it runs a `git.cmd` from the working
/// directory before the one on the path.
fn reads_only(action: &ApprovalAction) -> bool {
    let Some(PermissionActionDetails::Command { argv, .. }) = action.details() else {
        return false;
    };
    action.kind == ApprovalActionKind::ShellExecution
        && shell_script(argv).is_some_and(read_only_git)
}

/// The script a shell's command line runs, as the execution crate builds it.
fn shell_script(argv: &[String]) -> Option<&str> {
    match argv {
        [_, flag, script] if flag == "-c" || flag == "-lc" => Some(script),
        [_, no_logo, no_profile, non_interactive, command, script]
            if [no_logo, no_profile, non_interactive, command]
                == ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"] =>
        {
            script.strip_prefix(POWERSHELL_UTF8_OUTPUT_PREFIX)
        }
        _ => None,
    }
}

/// `git --no-pager <read-only command> <plain arguments>`: one command of
/// words made only of characters no shell gives a meaning to that could run
/// or write anything, so the words are what `git` receives.
fn read_only_git(script: &str) -> bool {
    let plain =
        |character: char| character.is_ascii_alphanumeric() || "-_./:=+,%~ \t".contains(character);
    if !script.chars().all(plain) {
        return false;
    }
    let words = script.split([' ', '\t']).filter(|word| !word.is_empty());
    match words.collect::<Vec<_>>().as_slice() {
        ["git", "--no-pager", command, arguments @ ..] => {
            READ_ONLY_GIT_COMMANDS.contains(command)
                && !arguments.iter().any(|argument| {
                    WRITING_OPTIONS
                        .iter()
                        .any(|option| argument.starts_with(option))
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
