use hanafubuki_agent_contracts::ToolCallValidationError;
use serde_json::{json, Value};

use super::{parse, ExecCall};
use crate::tool::ExecToolKind;

#[test]
fn execution_tools_are_typed_and_do_not_accept_executor_capabilities() {
    let (ExecCall::ExecCommand(request), _) = parse(
        ExecToolKind::ExecCommand,
        &json!({
            "cmd": "program argument",
            "shell": null,
            "login": null,
            "tty": null,
            "workdir": null,
            "yield_time_ms": 1000,
            "max_output_bytes": 4096
        }),
    )
    .unwrap() else {
        panic!("exec_command parses to a command");
    };
    assert_eq!(request.cmd, "program argument");
    assert!(request.login, "login defaults to true");

    // The executor's capabilities are never arguments: a key that names one
    // is ignored and changes nothing.
    for forbidden in ["argv", "executorId", "receipt", "confirmed"] {
        let arguments = json!({
            "cmd": "program",
            "shell": null,
            "login": null,
            "tty": null,
            "yield_time_ms": null,
            "max_output_bytes": null,
            "workdir": null
        });
        let mut forged = arguments.clone();
        forged[forbidden] = json!("forged");
        assert_eq!(
            parse(ExecToolKind::ExecCommand, &forged).unwrap(),
            parse(ExecToolKind::ExecCommand, &arguments).unwrap()
        );
    }
    for (kind, arguments) in [
        (
            ExecToolKind::WriteStdin,
            json!({ "session_id": 1, "chars": null, "yield_time_ms": null, "max_output_bytes": null }),
        ),
        (
            ExecToolKind::ResizeExecution,
            json!({ "session_id": 1, "cols": 80, "rows": 24 }),
        ),
    ] {
        let mut forged = arguments.clone();
        forged["receipt"] = json!("forged");
        assert_eq!(
            parse(kind, &forged).unwrap(),
            parse(kind, &arguments).unwrap()
        );
    }
    assert!(matches!(
        parse(
            ExecToolKind::ExecCommand,
            &json!({ "cmd": "x", "shell": "fish", "login": null, "tty": null, "yield_time_ms": null, "max_output_bytes": null, "workdir": null })
        ),
        Err(ToolCallValidationError::InvalidArgument("shell"))
    ));
    assert!(matches!(
        parse(ExecToolKind::ExecCommand, &json!({ "shell": null })),
        Err(ToolCallValidationError::MissingArgument("cmd"))
    ));
}

#[test]
fn write_stdin_default_wait_distinguishes_input_from_polling() {
    let wait = |chars: Value| {
        match parse(
        ExecToolKind::WriteStdin,
        &json!({ "session_id": 1, "chars": chars, "yield_time_ms": null, "max_output_bytes": null }),
    )
    .unwrap()
    .0
    {
        ExecCall::WriteStdin { yield_time_ms, .. } => yield_time_ms,
        _ => panic!("write_stdin parses to input"),
    }
    };
    assert_eq!(wait(json!("input")), 250);
    assert_eq!(wait(json!("")), 5_000);
    assert_eq!(wait(Value::Null), 5_000);
}

/// Strict function schemas list every property as required-and-nullable;
/// an omitted nullable field means the same as `null`.
#[test]
fn omitted_nullable_fields_read_as_null() {
    for (kind, arguments, nullable) in [
        (
            ExecToolKind::ExecCommand,
            json!({ "cmd": "program", "shell": null, "login": null, "tty": null, "yield_time_ms": null, "max_output_bytes": null, "workdir": null }),
            "workdir",
        ),
        (
            ExecToolKind::WriteStdin,
            json!({ "session_id": 1, "chars": null, "yield_time_ms": null, "max_output_bytes": null }),
            "max_output_bytes",
        ),
    ] {
        let complete = parse(kind, &arguments).unwrap();
        let mut missing = arguments;
        missing.as_object_mut().unwrap().remove(nullable);
        assert_eq!(parse(kind, &missing).unwrap(), complete);
    }
}

#[test]
fn out_of_range_waits_and_caps_are_clamped_with_a_note() {
    let (call, notes) = parse(
        ExecToolKind::WriteStdin,
        &json!({ "session_id": 1, "chars": "", "yield_time_ms": 120000, "max_output_bytes": 0 }),
    )
    .unwrap();
    assert_eq!(
        call,
        ExecCall::WriteStdin {
            session_id: 1,
            chars: String::new(),
            yield_time_ms: 30_000,
            max_output_bytes: Some(1),
        }
    );
    assert_eq!(
        notes.notes(),
        [
            "yield_time_ms 120000 is above its maximum; 30000 was used",
            "max_output_bytes 0 is below its minimum; 1 was used",
        ]
    );
    let (call, notes) = parse(
        ExecToolKind::ResizeExecution,
        &json!({ "session_id": 1, "cols": 900, "rows": 24 }),
    )
    .unwrap();
    assert_eq!(
        call,
        ExecCall::Resize {
            session_id: 1,
            cols: 500,
            rows: 24
        }
    );
    assert_eq!(notes.notes().len(), 1);
    assert!(matches!(
        parse(
            ExecToolKind::ResizeExecution,
            &json!({ "session_id": 1, "rows": 24 })
        ),
        Err(ToolCallValidationError::MissingArgument("cols"))
    ));
}
