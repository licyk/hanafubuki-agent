use hanafubuki_agent_contracts::ToolResult;
use serde_json::json;

use super::*;

fn exec_output() -> ToolOutput {
    ToolResult::success(json!({
        "type": "unified_exec",
        "output": "running 3 tests\ntest result: FAILED\n",
        "exitCode": 101,
        "sessionId": null,
        "wallTimeMs": 4200,
        "truncated": false
    }))
    .output
}

#[test]
fn a_command_shows_its_line_and_what_it_printed() {
    let arguments = json!({ "cmd": "cargo test -p foo", "workdir": "crates/foo", "tty": null });

    assert_eq!(
        ExecPresenter.subject(EXEC_COMMAND, &arguments).as_deref(),
        Some("cargo test -p foo")
    );
    assert_eq!(
        ExecPresenter.display(EXEC_COMMAND, &arguments, &exec_output()),
        Some(AgentToolDisplay::Command {
            command: "cargo test -p foo".to_string(),
            cwd: Some("crates/foo".to_string()),
            output: "running 3 tests\ntest result: FAILED\n".to_string(),
            exit_code: Some(101),
            duration_millis: Some(4200),
            truncated: false,
            styled: None,
        })
    );
}

#[test]
fn input_names_its_session_and_escapes_what_was_typed() {
    let interrupt = json!({ "session_id": 3, "chars": "\u{3}" });
    let poll = json!({ "session_id": 3, "chars": null });

    assert_eq!(
        ExecPresenter.subject(WRITE_STDIN, &interrupt).as_deref(),
        Some("session 3 ← \\u{3}")
    );
    assert_eq!(
        ExecPresenter.subject(WRITE_STDIN, &poll).as_deref(),
        Some("session 3")
    );
    let Some(AgentToolDisplay::Command {
        command, output, ..
    }) = ExecPresenter.display(WRITE_STDIN, &poll, &exec_output())
    else {
        panic!("a poll shows the command's output");
    };
    assert_eq!(command, "");
    assert!(output.starts_with("running 3 tests"));
}

#[test]
fn a_resize_has_a_subject_and_what_cannot_be_read_takes_the_fallback() {
    let resize = json!({ "session_id": 3, "cols": 120, "rows": 40 });
    assert_eq!(
        ExecPresenter.subject(RESIZE_EXECUTION, &resize).as_deref(),
        Some("session 3 to 120×40")
    );
    assert_eq!(
        ExecPresenter.display(RESIZE_EXECUTION, &resize, &exec_output()),
        None
    );
    // An error the tool returned is no command result.
    let failed = ToolResult::model_visible_error(json!({ "error": "denied" })).output;
    let arguments = json!({ "cmd": "rm -rf target" });
    assert_eq!(
        ExecPresenter.display(EXEC_COMMAND, &arguments, &failed),
        None
    );
    assert_eq!(
        ExecPresenter.subject(EXEC_COMMAND, &json!({ "cmd": 1 })),
        None
    );
}
