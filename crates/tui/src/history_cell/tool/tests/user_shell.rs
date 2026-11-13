//! A command the person typed (`!command`): `Running` while it runs, as
//! any command, then `You ran`; a failed one adds that `!` commands run in
//! the session's sandbox; one its turn's end overtook is written as it
//! stood.

use super::{both, command, render, shown};
use crate::history_cell::tool::{new_tool_call, new_user_shell_command, ToolCallCell};
use hanafubuki_agent::AgentToolCallDisplay;

fn typed(display: hanafubuki_agent::AgentToolDisplay) -> ToolCallCell {
    shown("exec_command", None, display).typed_by_user()
}

#[test]
fn a_finished_user_shell_command_snapshot() {
    let cell = typed(command("ls", "a.txt\nb.txt\n", Some(0)));
    insta::assert_snapshot!(both(&cell, 60));
}

#[test]
fn a_failed_user_shell_command_snapshot() {
    let cell = typed(command(
        "npm install",
        "npm ERR! network request failed\n",
        Some(1),
    ));
    insta::assert_snapshot!(both(&cell, 60));
}

#[test]
fn a_running_user_shell_command_shows_what_it_printed() {
    let mut cell = new_tool_call(
        "user-shell-1".to_string(),
        "exec_command".to_string(),
        Some("cargo build".to_string()),
    )
    .typed_by_user();
    cell.push_output("user-shell-1", "Compiling foo\n", false);
    let rendered = render(&cell, 60);
    assert!(rendered.contains(" Running cargo build"), "{rendered}");
    assert!(rendered.contains("└ Compiling foo"), "{rendered}");

    // Interrupted: the turn's end writes it as it stood.
    cell.end_running();
    let rendered = render(&cell, 60);
    assert!(rendered.contains("• You ran cargo build"), "{rendered}");
}

#[test]
fn a_replayed_user_shell_command_is_the_persons() {
    let cell = new_user_shell_command(
        "user-shell-7".to_string(),
        AgentToolCallDisplay {
            subject: Some("git status".to_string()),
            display: Some(command("git status", "nothing to commit\n", Some(0))),
        },
    );
    let rendered = render(&cell, 60);
    assert!(rendered.contains("• You ran git status"), "{rendered}");
    assert!(rendered.contains("nothing to commit"), "{rendered}");
}
