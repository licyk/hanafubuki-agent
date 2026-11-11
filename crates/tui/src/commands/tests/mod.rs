use super::*;

#[test]
fn every_command_is_found_by_its_name_and_its_aliases() {
    for command in COMMANDS {
        for name in command.names() {
            assert_eq!(find(name), Some(command), "/{name}");
        }
    }
    assert_eq!(find("quit"), Some(&exit::EXIT));
    assert_eq!(find("clean"), Some(&terminals::STOP));
    assert_eq!(find("cwd"), Some(&pwd::PWD));
    assert_eq!(find("cd"), Some(&cd::CD));
    assert_eq!(find("mod"), None);
}

#[test]
fn names_are_unique_across_commands() {
    let mut names: Vec<&str> = COMMANDS.into_iter().flat_map(CommandSpec::names).collect();
    let all = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all, "{names:?}");
}

#[test]
fn the_table_keeps_the_presentation_order() {
    let names: Vec<&str> = COMMANDS.into_iter().map(|command| command.name).collect();
    assert_eq!(
        names,
        [
            "model",
            "providers",
            "mode",
            "permissions",
            "approve",
            "setup-default-sandbox",
            "skills",
            "rename",
            "new",
            "archive",
            "delete",
            "resume",
            "fork",
            "worktree",
            "init",
            "compact",
            "compression",
            "recap",
            "review",
            "side",
            "subagents",
            "agents",
            "goal",
            "copy",
            "export",
            "diff",
            "mention",
            "status",
            "pwd",
            "cd",
            "plugins",
            "mcp",
            "hooks",
            "raw",
            "theme",
            "statusline",
            "title",
            "tui",
            "config",
            "keymap",
            "presets",
            "memories",
            "warnings",
            "ps",
            "stop",
            "clear",
            "exit"
        ]
    );
}

#[test]
fn availability_follows_the_spec() {
    let during_task: Vec<&str> = COMMANDS
        .into_iter()
        .filter(|command| command.during_task)
        .map(|command| command.name)
        .collect();
    assert_eq!(
        during_task,
        [
            "permissions",
            "approve",
            "setup-default-sandbox",
            "skills",
            "compression",
            "review",
            "side",
            "subagents",
            "agents",
            "goal",
            "copy",
            "diff",
            "mention",
            "status",
            "pwd",
            "plugins",
            "mcp",
            "hooks",
            "raw",
            "theme",
            "statusline",
            "title",
            "tui",
            "config",
            "keymap",
            "presets",
            "memories",
            "warnings",
            "ps",
            "stop",
            "exit"
        ]
    );
    let needs_session: Vec<&str> = COMMANDS
        .into_iter()
        .filter(|command| command.needs_session)
        .map(|command| command.name)
        .collect();
    assert_eq!(
        needs_session,
        [
            "model",
            "mode",
            "permissions",
            "rename",
            "compact",
            "goal",
            "status"
        ]
    );
    let inline_args: Vec<&str> = COMMANDS
        .into_iter()
        .filter(|command| command.inline_args)
        .map(|command| command.name)
        .collect();
    assert_eq!(
        inline_args,
        [
            "providers",
            "permissions",
            "rename",
            "resume",
            "fork",
            "compression",
            "review",
            "side",
            "goal",
            "export",
            "cd",
            "mcp",
            "raw",
            "statusline",
            "title",
            "config",
            "keymap",
            "clear"
        ]
    );
}

#[test]
fn a_prefix_is_a_case_insensitive_subsequence_of_an_enabled_command() {
    assert!(has_prefix("mo", &[]));
    assert!(has_prefix("MCP", &[]));
    assert!(has_prefix("rsm", &[]));
    assert!(has_prefix("qu", &[]));
    assert!(!has_prefix("zz", &[]));
    assert!(!has_prefix("rsm", &["resume", "rename"]));
    assert!(!has_prefix("qu", &["exit"]));
    assert_eq!(has_prefix("setup-default", &[]), cfg!(windows));
}

/// The chat and what it sent, for the commands' own tests too.
pub(super) mod session;
mod workspace;
