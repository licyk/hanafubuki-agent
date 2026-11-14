//! What `exec_command` returns of a real command's output, through the real
//! executor: the hold-back of an unfinished sequence, the output bound and its
//! marker (the plan's §4.9 steps 7 and 9), and a PTY's screen across polls.

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
        timeout_ms: 600_000,
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
    polls_then(request, yield_ms, yield_ms)
}

/// [`polls`], the first poll yielding after `first_ms` and every later one
/// after `then_ms`.
fn polls_then(request: ExecCommandRequest, first_ms: u64, then_ms: u64) -> Vec<UnifiedExecResult> {
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
            first_ms,
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
                        yield_time_ms: then_ms,
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
fn a_listing_over_the_output_bound_says_what_it_left_out() {
    assert_every_line_or_a_marker(&polls(request(LISTING, false, Some(4096)), 10_000), 300);
}

#[test]
fn a_slow_listing_over_the_output_bound_says_what_it_left_out() {
    // As reported from `dir`: the head arrives before the first poll, the
    // rest after it, more than `max_output_bytes`, which used to bound the
    // buffer too, so the next poll's offset was gone by then.
    let slow = format!(
        "printf 'line 1 of the listing\\n'; sleep 2; i=2; {}",
        &LISTING[4..]
    );
    let results = polls(request(&slow, false, Some(4096)), 500);
    assert!(results.len() > 1, "one poll saw everything");
    assert_every_line_or_a_marker(&results, 300);
    assert_lines_whole_and_gaps_marked(&results, 300);
}

#[test]
fn a_slow_listing_within_the_default_bound_arrives_whole() {
    let slow = format!(
        "printf 'line 1 of the listing\\n'; sleep 2; i=2; {}",
        &LISTING[4..]
    );
    let results = polls(request(&slow, false, None), 500);
    assert!(results.len() > 1, "one poll saw everything");
    assert!(results.iter().all(|result| !result.truncated));
    assert_lines_whole_and_gaps_marked(&results, 300);
}

#[test]
fn a_listing_over_the_output_bound_keeps_whole_lines_around_the_marker() {
    for tty in [false, true] {
        let results = polls(request(LISTING, tty, Some(4096)), 10_000);
        assert!(results.iter().any(|result| result.truncated));
        assert_lines_whole_and_gaps_marked(&results, 300);
    }
}

#[test]
fn a_poll_behind_the_retained_output_says_exactly_what_it_missed() {
    // More than the 4 MiB a session keeps arrives between two polls, the
    // second once the command has finished.
    let results = polls_then(
        request(
            "printf 'first\\n'; sleep 2; awk 'BEGIN { for (i = 1; i <= 1000000; i++) print i }'",
            false,
            None,
        ),
        1_000,
        30_000,
    );
    assert!(results.len() > 1, "one poll saw everything");
    assert_eq!(results[0].output, "first\n");
    let rest = joined(&results[1..]);
    let mut lines = rest.lines();
    let omitted = marker_bytes(lines.next().unwrap()).expect("starts with the marker");
    let first_kept: u64 = lines.next().unwrap().parse().unwrap();
    // Everything between `first` and the line kept is counted.
    let missed: u64 = (1..first_kept)
        .map(|n| n.to_string().len() as u64 + 1)
        .sum();
    assert_eq!(omitted, missed);
    let kept = std::iter::once(first_kept)
        .chain(lines.map(|line| line.parse::<u64>().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(kept, (first_kept..=1_000_000).collect::<Vec<_>>());
    assert!(results[1..].iter().any(|result| result.truncated));
}

#[test]
fn a_pty_prompt_and_its_answer_arrive_once_each() {
    let service = AgentExecutionService::new(ExecutorManager::default());
    let scope = AgentExecutionScope::for_agent_session(62).unwrap();
    let request = request(
        "printf 'Name: '; read name; printf 'hello %s\\n' \"$name\"",
        true,
        None,
    );
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let prompt = service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            2_000,
            None,
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(prompt.output, "Name:");
    let answered = service
        .write_stdin(
            &scope,
            WriteStdinRequest {
                session_id: started.execution_id,
                chars: "bob\n".to_string(),
                yield_time_ms: 10_000,
                max_output_bytes: None,
            },
            &CancellationToken::default(),
        )
        .unwrap();
    // The prompt's row, now with the echoed answer, then the reply.
    assert_eq!(answered.output, "Name: bob\nhello bob\n");
    assert_eq!(answered.exit_code, Some(0));
}

/// The size a marker line names, if `line` is one.
fn marker_bytes(line: &str) -> Option<u64> {
    line.strip_prefix("[… ")?
        .strip_suffix(" bytes omitted …]")?
        .parse()
        .ok()
}

/// Every line is a whole numbered line or a marker, and the numbers count up
/// by one except across a marker: no line is glued to another, and none is
/// left out without one.
fn assert_lines_whole_and_gaps_marked(results: &[UnifiedExecResult], lines: usize) {
    let output = joined(results);
    let mut expected = 1;
    let mut after_marker = false;
    for line in output.lines() {
        if marker_bytes(line).is_some() {
            after_marker = true;
            continue;
        }
        let number = line
            .strip_prefix("line ")
            .and_then(|rest| rest.strip_suffix(" of the listing"))
            .and_then(|number| number.parse::<usize>().ok())
            .unwrap_or_else(|| panic!("not a whole line: {line:?} in {output:?}"));
        assert!(
            number == expected || (after_marker && number > expected),
            "line {number} after {} in {output:?}",
            expected - 1
        );
        expected = number + 1;
        after_marker = false;
    }
    assert_eq!(expected, lines + 1, "{output:?}");
}
