// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::user_shell_command_record;

/// Codex's `formats_basic_record` and `uses_aggregated_output_over_streams`.
#[test]
fn renders_codex_record() {
    assert_eq!(
        user_shell_command_record("echo hi", Some(0), 1_000, "hi"),
        "<user_shell_command>\n<command>\necho hi\n</command>\n<result>\nExit code: 0\nDuration: 1.0000 seconds\nOutput:\nhi\n</result>\n</user_shell_command>"
    );
    assert_eq!(
        user_shell_command_record("false", Some(42), 120, "combined output wins"),
        "<user_shell_command>\n<command>\nfalse\n</command>\n<result>\nExit code: 42\nDuration: 0.1200 seconds\nOutput:\ncombined output wins\n</result>\n</user_shell_command>"
    );
}

#[test]
fn a_command_without_an_exit_says_none() {
    assert_eq!(
        user_shell_command_record("sleep 30", None, 2_500, "the command was stopped"),
        "<user_shell_command>\n<command>\nsleep 30\n</command>\n<result>\nExit code: none\nDuration: 2.5000 seconds\nOutput:\nthe command was stopped\n</result>\n</user_shell_command>"
    );
}
