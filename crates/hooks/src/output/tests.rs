//! What a hook's exit code and output mean for each event.

use super::*;

fn exited(code: i32, stdout: &str, stderr: &str) -> CommandRun {
    CommandRun {
        exit_code: Some(code),
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        error: None,
    }
}

#[test]
fn a_pre_tool_use_hook_denies_by_decision_or_exit_code_2() {
    let specific = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"no rm"}}"#;
    assert_eq!(
        interpret(HookEventName::PreToolUse, &exited(0, specific, "")),
        Ok(Effect::Deny("no rm".to_string()))
    );
    let legacy = r#"{"decision":"block","reason":"not here","somethingNew":1}"#;
    assert_eq!(
        interpret(HookEventName::PreToolUse, &exited(0, legacy, "")),
        Ok(Effect::Deny("not here".to_string()))
    );
    assert_eq!(
        interpret(HookEventName::PreToolUse, &exited(2, "", " blocked \n")),
        Ok(Effect::Deny("blocked".to_string()))
    );
    assert!(interpret(HookEventName::PreToolUse, &exited(2, "", "")).is_err());
    assert!(interpret(
        HookEventName::PreToolUse,
        &exited(0, r#"{"decision":"block"}"#, "")
    )
    .is_err());
}

#[test]
fn output_is_read_tolerantly() {
    // A field of the wrong type is absent; plain text is no decision.
    assert_eq!(
        interpret(
            HookEventName::PreToolUse,
            &exited(0, r#"{"decision":7,"hookSpecificOutput":[]}"#, "")
        ),
        Ok(Effect::None)
    );
    assert_eq!(
        interpret(HookEventName::PreToolUse, &exited(0, "checked", "")),
        Ok(Effect::None)
    );
    assert!(interpret(HookEventName::PreToolUse, &exited(0, "{not json", "")).is_err());
    assert!(interpret(HookEventName::PreToolUse, &exited(1, "", "")).is_err());
}

#[test]
fn context_comes_from_additional_context_or_plain_stdout() {
    let post =
        r#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"lint: ok"}}"#;
    assert_eq!(
        interpret(HookEventName::PostToolUse, &exited(0, post, "")),
        Ok(Effect::Context("lint: ok".to_string()))
    );
    assert_eq!(
        interpret(
            HookEventName::UserPromptSubmit,
            &exited(0, "today is Monday\n", "")
        ),
        Ok(Effect::Context("today is Monday".to_string()))
    );
    assert_eq!(
        interpret(HookEventName::PostToolUse, &exited(0, "plain", "")),
        Ok(Effect::None)
    );
    assert!(interpret(
        HookEventName::UserPromptSubmit,
        &exited(0, r#"{"decision":"block","reason":"secret"}"#, "")
    )
    .is_err());
}

#[test]
fn a_stop_hook_continues_with_its_reason_or_stops() {
    assert_eq!(
        interpret(
            HookEventName::Stop,
            &exited(0, r#"{"decision":"block","reason":"run the tests"}"#, "")
        ),
        Ok(Effect::Continue("run the tests".to_string()))
    );
    assert_eq!(
        interpret(HookEventName::Stop, &exited(0, r#"{"continue":false}"#, "")),
        Ok(Effect::Stop)
    );
    assert_eq!(
        interpret(HookEventName::Stop, &exited(2, "", "again")),
        Ok(Effect::Continue("again".to_string()))
    );
}

#[test]
fn a_run_without_a_result_has_no_effect() {
    let timed_out = CommandRun {
        error: Some("the hook timed out after 1 s".to_string()),
        ..CommandRun::default()
    };
    assert_eq!(
        interpret(HookEventName::PreToolUse, &timed_out),
        Err("the hook timed out after 1 s".to_string())
    );
}
