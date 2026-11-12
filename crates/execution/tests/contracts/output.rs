//! What `exec_command` returns of a real command's output, through the real
//! executor: the hold-back of an unfinished sequence and the output bound
//! (the plan's §4.9 steps 7 and 9).

use super::*;

use hanafubuki_agent_execution::{AgentExecutionId, ExecCommandRequest, WriteStdinRequest};
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::SandboxSettings;

fn request(cmd: &str, tty: bool, max_output_bytes: Option<usize>) -> ExecCommandRequest {
    ExecCommandRequest {
        cmd: cmd.to_string(),
        shell: None,
        login: false,
        tty,
        yield_time_ms: 10_000,
        max_output_bytes,
        workdir: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::new(),
        origin: ApprovalOrigin::Model,
        sandbox: SandboxSettings::full_access(),
        sandbox_escalation: None,
    }
}

/// Starts `request` and polls it as the model does, every `yield_ms`, until it
/// ends; returns each poll's result.
fn polls(request: ExecCommandRequest, yield_ms: u64) -> Vec<UnifiedExecResult> {
    let service = AgentExecutionService::new(ExecutorManager::default());
    let scope = AgentExecutionScope::for_agent_session(61).unwrap();
    let action = service.prepare_exec_command_action(&request).unwrap();
    let max_output_bytes = request.max_output_bytes;
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
            yield_ms,
            max_output_bytes,
            &CancellationToken::default(),
        )
        .unwrap()];
    while let Some(session_id) = results.last().unwrap().session_id {
        assert!(results.len() < 200, "never finished");
        results.push(
            service
                .write_stdin(
                    &scope,
                    WriteStdinRequest {
                        session_id: AgentExecutionId::from_untrusted(session_id).unwrap(),
                        chars: String::new(),
                        yield_time_ms: yield_ms,
                        max_output_bytes,
                    },
                    &CancellationToken::default(),
                )
                .unwrap(),
        );
    }
    results
}

fn joined(results: &[UnifiedExecResult]) -> String {
    results
        .iter()
        .map(|result| result.output.as_str())
        .collect()
}

/// Every numbered line arrived whole, or the output says what it left out.
fn assert_every_line_or_a_marker(results: &[UnifiedExecResult], lines: usize) {
    let output = joined(results);
    let truncated = results.iter().any(|result| result.truncated);
    let missing = (1..=lines)
        .filter(|number| {
            let line = format!("line {number} of the listing");
            !output.lines().any(|shown| shown.trim_end() == line)
        })
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty() || (truncated && output.contains("omitted")),
        "missing {missing:?} (truncated: {truncated}) in {output:?}"
    );
}

const LISTING: &str =
    "i=1; while [ $i -le 300 ]; do printf 'line %d of the listing\\n' $i; i=$((i+1)); done";

#[test]
#[ignore = "reproduces §4.9 step 7, the hold-back skipped after U+FFFD; fixed in E3"]
fn an_unfinished_title_holding_invalid_bytes_is_held_for_the_next_poll() {
    // A title (OSC) whose text is not UTF-8 is cut by the poll: its rest must
    // not print as text on the next one.
    let results = polls(
        request(
            "printf 'start\\n\\033]0;\\377ti'; sleep 2; printf 'tle\\007after\\n'",
            false,
            None,
        ),
        500,
    );
    assert!(results.len() > 1, "one poll saw everything");
    assert_eq!(joined(&results), "start\nafter\n");
}

#[test]
#[ignore = "reproduces §4.9 step 9, silent loss past the read bound; fixed in E3"]
fn a_finished_command_past_the_read_bound_returns_head_tail_and_truncated() {
    // 100 KiB, within the 256 KiB buffer and over the 64 KiB a read returns.
    let results = polls(
        request(
            "printf 'head-start\\n'; head -c 102400 /dev/zero | tr '\\0' x; printf '\\ntail-end\\n'",
            false,
            None,
        ),
        10_000,
    );
    let [result] = results.as_slice() else {
        panic!("still running after one poll");
    };
    assert!(
        result.truncated,
        "{:?}",
        &result.output[result.output.len() - 32..]
    );
    assert!(result.output.starts_with("head-start\n"));
    assert!(result.output.ends_with("tail-end\n"));
}

#[test]
fn a_long_listing_arrives_whole_through_a_pipe() {
    assert_every_line_or_a_marker(&polls(request(LISTING, false, None), 10_000), 300);
}

#[test]
fn a_long_listing_arrives_whole_through_a_pty() {
    assert_every_line_or_a_marker(&polls(request(LISTING, true, None), 10_000), 300);
}

#[test]
fn a_long_listing_polled_while_it_runs_arrives_whole() {
    let slow = "i=1; while [ $i -le 300 ]; do printf 'line %d of the listing\\n' $i; \
                [ $((i % 50)) -eq 0 ] && sleep 0.2; i=$((i+1)); done";
    assert_every_line_or_a_marker(&polls(request(slow, false, None), 100), 300);
}

#[test]
#[ignore = "reproduces §4.9 step 9, no marker past max_output_bytes; fixed in E3"]
fn a_listing_over_the_output_bound_says_what_it_left_out() {
    assert_every_line_or_a_marker(&polls(request(LISTING, false, Some(4096)), 10_000), 300);
}

#[test]
#[ignore = "reproduces the reported mid-listing loss: a poll past trimmed output has no marker; fixed in E3"]
fn a_slow_listing_over_the_output_bound_says_what_it_left_out() {
    // As reported from `dir`: the head arrives before the first poll, the
    // rest after it, more than `max_output_bytes`, which also bounds the
    // buffer; the next poll's offset is gone by then.
    let slow = format!(
        "printf 'line 1 of the listing\\n'; sleep 2; i=2; {}",
        &LISTING[4..]
    );
    let results = polls(request(&slow, false, Some(4096)), 500);
    assert!(results.len() > 1, "one poll saw everything");
    assert_every_line_or_a_marker(&results, 300);
}
