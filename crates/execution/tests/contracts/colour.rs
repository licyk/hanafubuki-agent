//! Colour in a real command's output, through the real executor, in pipe
//! and PTY mode on every platform: the result the model reads is plain
//! text, and its styles (for a display, never serialized) say where the
//! colour was. The commands print their own escape sequences, as a program
//! forcing colour does: the environment the Agent gives commands
//! (`NO_COLOR=1`, `TERM=dumb`) is unchanged.

use super::*;

use hanafubuki_agent_execution::{AgentExecutionId, ExecCommandRequest, WriteStdinRequest};
use hanafubuki_agent_foundation::terminal_text::TextColor;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::SandboxSettings;

/// Long enough for the slowest shell start the CI runners have shown.
const YIELD_MS: u64 = 30_000;

/// `red` in red, then ` plain`, on one line.
#[cfg(unix)]
const RED_THEN_PLAIN: &str = r"printf '\033[31mred\033[0m plain\n'";
#[cfg(windows)]
const RED_THEN_PLAIN: &str = r#"[Console]::Out.Write("$([char]27)[31mred$([char]27)[0m plain`n")"#;

fn request(cmd: &str, tty: bool, yield_time_ms: u64) -> ExecCommandRequest {
    ExecCommandRequest {
        cmd: cmd.to_string(),
        shell: None,
        login: false,
        tty,
        yield_time_ms,
        max_output_bytes: None,
        timeout_ms: 600_000,
        workdir: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::new(),
        origin: ApprovalOrigin::Model,
        sandbox: SandboxSettings::full_access(),
        sandbox_escalation: None,
    }
}

/// Runs `request` to its end, polling as the model does every `then_ms`
/// after the first poll; returns each poll's result.
fn polls(request: ExecCommandRequest, then_ms: u64) -> Vec<UnifiedExecResult> {
    let service = AgentExecutionService::new(ExecutorManager::default());
    let scope = AgentExecutionScope::for_agent_session(71).unwrap();
    let action = service.prepare_exec_command_action(&request).unwrap();
    let first_ms = request.yield_time_ms;
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let mut results = vec![service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            first_ms,
            None,
            &CancellationToken::default(),
        )
        .unwrap()];
    while let Some(session_id) = results.last().unwrap().session_id {
        assert!(results.len() < 100, "never finished");
        results.push(
            service
                .write_stdin(
                    &scope,
                    WriteStdinRequest {
                        session_id: AgentExecutionId::from_untrusted(session_id).unwrap(),
                        chars: String::new(),
                        yield_time_ms: then_ms,
                        max_output_bytes: None,
                    },
                    &CancellationToken::default(),
                )
                .unwrap(),
        );
    }
    results
}

/// The text of each styled run of `result`, with its foreground.
fn runs(result: &UnifiedExecResult) -> Vec<(String, Option<TextColor>)> {
    let Some(styles) = &result.styles else {
        return Vec::new();
    };
    assert!(
        styles.fits(&result.output),
        "{styles:?} {:?}",
        result.output
    );
    styles
        .spans
        .iter()
        .map(|span| {
            (
                result.output[span.start..span.end].to_string(),
                span.style.fg,
            )
        })
        .collect()
}

fn assert_plain_for_the_model(result: &UnifiedExecResult) {
    let wire = serde_json::to_string(result).unwrap();
    assert!(!wire.contains("\\u001b"), "{wire}");
    assert!(!wire.contains("styles"), "{wire}");
    assert!(!result.output.contains('\u{1b}'), "{:?}", result.output);
}

#[test]
fn a_coloured_word_is_plain_for_the_model_and_styled_for_a_display_in_pipe_mode() {
    let results = polls(request(RED_THEN_PLAIN, false, YIELD_MS), YIELD_MS);
    let result = results.last().unwrap();
    assert_plain_for_the_model(result);
    assert_eq!(result.output.trim_end(), "red plain");
    assert_eq!(
        runs(result),
        vec![("red".to_string(), Some(TextColor::Indexed(1)))]
    );
}

#[test]
fn a_coloured_word_is_plain_for_the_model_and_styled_for_a_display_in_pty_mode() {
    let results = polls(request(RED_THEN_PLAIN, true, YIELD_MS), YIELD_MS);
    let output = results
        .iter()
        .map(|result| result.output.as_str())
        .collect::<String>();
    for result in &results {
        assert_plain_for_the_model(result);
    }
    assert!(
        output.lines().any(|line| line.trim_end() == "red plain"),
        "{output:?}"
    );
    // ConPTY repaints in its own sequences, so the colour is read back
    // from the screen's cells rather than compared byte for byte.
    let runs = results.iter().flat_map(runs).collect::<Vec<_>>();
    assert!(
        runs.iter()
            .any(|(text, fg)| text.contains("red") && fg.is_some()),
        "{runs:?} in {output:?}"
    );
    assert!(
        runs.iter().all(|(text, _)| !text.contains("plain")),
        "{runs:?}"
    );
}

/// A style still in force when a poll returns goes on in the next one.
#[cfg(unix)]
#[test]
fn a_style_left_open_by_one_poll_goes_on_in_the_next() {
    let results = polls(
        request(
            r"printf '\033[32mfirst\n'; sleep 2; printf 'second\033[0m third\n'",
            false,
            500,
        ),
        YIELD_MS,
    );
    assert!(results.len() > 1, "one poll saw everything");
    let joined = results
        .iter()
        .map(|result| result.output.as_str())
        .collect::<String>();
    assert_eq!(joined, "first\nsecond third\n");
    let green = Some(TextColor::Indexed(2));
    let runs = results.iter().flat_map(runs).collect::<Vec<_>>();
    assert_eq!(
        runs,
        vec![("first".to_string(), green), ("second".to_string(), green)]
    );
}

/// The styles across the gap a bounded poll leaves: each side keeps its
/// own, and the marker is plain.
#[cfg(unix)]
#[test]
fn styles_stay_with_their_text_on_both_sides_of_the_omission_marker() {
    let script = r"i=1; while [ $i -le 400 ]; do printf '\033[34mline %d\033[0m of the listing\n' $i; i=$((i+1)); done";
    let mut request = request(script, false, YIELD_MS);
    request.max_output_bytes = Some(2048);
    let service = AgentExecutionService::new(ExecutorManager::default());
    let scope = AgentExecutionScope::for_agent_session(72).unwrap();
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let result = service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            YIELD_MS,
            Some(2048),
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(result.truncated);
    assert!(result.output.contains("bytes omitted"), "{}", result.output);
    let styles = result.styles.as_ref().unwrap();
    assert!(styles.fits(&result.output));
    for span in &styles.spans {
        let text = &result.output[span.start..span.end];
        assert!(text.starts_with("line "), "{text:?}");
        assert_eq!(span.style.fg, Some(TextColor::Indexed(4)));
    }
    for line in result.output.lines() {
        let styled = styles.spans.iter().any(|span| {
            let start = line.as_ptr() as usize - result.output.as_ptr() as usize;
            span.start >= start && span.end <= start + line.len()
        });
        assert_eq!(styled, line.starts_with("line "), "{line:?}");
    }
}
