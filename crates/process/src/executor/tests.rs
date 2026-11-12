use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use super::manager::*;
use super::model::{
    EnvironmentInheritance, ExecutorInfo, ExecutorIoMode, ExecutorStartRequest, ExecutorStatus,
    OutputStream,
};
use super::ExecutorManager;
use crate::ownership::ProcessOwnerId;

fn req(command: &[&str]) -> ExecutorStartRequest {
    ExecutorStartRequest {
        executor_id: None,
        command: command.iter().map(|s| s.to_string()).collect(),
        cwd: None,
        environment_inheritance: EnvironmentInheritance::Inherit,
        env: std::collections::HashMap::new(),
        io_mode: ExecutorIoMode::Pipe,
        stream_stdin: false,
        stream_output: true,
        cols: None,
        rows: None,
        timeout_ms: None,
        output_bytes_cap: None,
    }
}

fn wait_terminal(manager: &ExecutorManager, id: &str) -> super::model::ExecutorOutput {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let out = read_executor_output(manager, id, Some(0), None).unwrap();
        if out.status.is_terminal() {
            return out;
        }
        assert!(Instant::now() < deadline, "executor did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn merged_text(out: &super::model::ExecutorOutput, stream: OutputStream) -> String {
    let mut bytes = Vec::new();
    for chunk in &out.chunks {
        if chunk.stream == stream {
            bytes.extend_from_slice(&chunk.data);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn executor_io_mode_contract_defaults_to_pipe_and_ignores_the_removed_boolean() {
    let defaulted: ExecutorStartRequest = serde_json::from_value(serde_json::json!({
        "command": ["tool"],
        "environmentInheritance": "inherit"
    }))
    .unwrap();
    assert_eq!(defaulted.io_mode, ExecutorIoMode::Pipe);

    let terminal: ExecutorStartRequest = serde_json::from_value(serde_json::json!({
        "command": ["tool"],
        "environmentInheritance": "inherit",
        "ioMode": "pty"
    }))
    .unwrap();
    assert_eq!(terminal.io_mode, ExecutorIoMode::Pty);

    let removed: ExecutorStartRequest = serde_json::from_value(serde_json::json!({
        "command": ["tool"],
        "environmentInheritance": "inherit",
        "pty": true
    }))
    .unwrap();
    assert_eq!(removed.io_mode, ExecutorIoMode::Pipe);

    let encoded = serde_json::to_value(ExecutorInfo {
        executor_id: "exec-1".to_string(),
        status: ExecutorStatus::Running,
        io_mode: ExecutorIoMode::Pty,
        command: vec!["tool".to_string()],
        exit_code: None,
        error: None,
        output_offset: 0,
    })
    .unwrap();
    assert_eq!(encoded["ioMode"], "pty");
    assert!(encoded.get("pty").is_none());
}

#[test]
fn rejects_empty_command() {
    let manager = ExecutorManager::default();
    assert!(start_executor(&manager, req(&[])).is_err());
    assert!(start_executor(&manager, req(&["   "])).is_err());
}

#[test]
fn independent_start_accepts_pipe() {
    let manager = ExecutorManager::default();
    let info =
        start_independent_executor(&manager, req(&["sh", "-c", "printf independent"])).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert_eq!(out.status, ExecutorStatus::Exited);
    assert!(merged_text(&out, OutputStream::Stdout).contains("independent"));
    dispose_executor(&manager, &info.executor_id).unwrap();
}

#[test]
fn simple_command_exits_zero_with_stdout() {
    let manager = ExecutorManager::default();
    let info = start_executor(&manager, req(&["sh", "-c", "printf hello-exec"])).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert_eq!(out.status, ExecutorStatus::Exited);
    assert_eq!(out.exit_code, Some(0));
    assert!(merged_text(&out, OutputStream::Stdout).contains("hello-exec"));
}

#[test]
fn failing_command_returns_nonzero() {
    let manager = ExecutorManager::default();
    let info = start_executor(&manager, req(&["sh", "-c", "exit 7"])).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert_eq!(out.exit_code, Some(7));
}

#[test]
fn stderr_is_captured_separately_in_pipe_mode() {
    let manager = ExecutorManager::default();
    let info = start_executor(&manager, req(&["sh", "-c", "printf oops 1>&2"])).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert!(merged_text(&out, OutputStream::Stderr).contains("oops"));
    assert!(merged_text(&out, OutputStream::Stdout).is_empty());
}

#[test]
fn environment_override_is_applied() {
    let manager = ExecutorManager::default();
    let mut request = req(&["sh", "-c", "printf %s \"$HANAFUBUKI_EXEC_TEST\""]);
    request.env.insert(
        "HANAFUBUKI_EXEC_TEST".to_string(),
        Some("env-ok".to_string()),
    );
    let info = start_executor(&manager, request).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert!(merged_text(&out, OutputStream::Stdout).contains("env-ok"));
}

#[cfg(unix)]
#[test]
fn clear_environment_is_honored_by_pipe_and_pty_spawns() {
    for io_mode in [ExecutorIoMode::Pipe, ExecutorIoMode::Pty] {
        let manager = ExecutorManager::default();
        let mut request = req(&["/usr/bin/env"]);
        request.io_mode = io_mode;
        request.environment_inheritance = EnvironmentInheritance::Clear;
        request.env.insert(
            "HANAFUBUKI_CLEAN_ENV_SENTINEL".to_string(),
            Some("present".to_string()),
        );
        let info = start_executor(&manager, request).unwrap();
        let output = wait_terminal(&manager, &info.executor_id);
        let stream = if io_mode.uses_pty() {
            OutputStream::Merged
        } else {
            OutputStream::Stdout
        };
        let text = merged_text(&output, stream);
        assert!(
            text.contains("HANAFUBUKI_CLEAN_ENV_SENTINEL=present"),
            "{text:?}"
        );
        assert!(
            !text.lines().any(|line| line.starts_with("PATH=")),
            "{text:?}"
        );
    }
}

#[test]
fn native_environment_reaches_pipe_and_pty_without_assignment_argv() {
    for io_mode in [ExecutorIoMode::Pipe, ExecutorIoMode::Pty] {
        let manager = ExecutorManager::default();
        #[cfg(unix)]
        let mut request = req(&["/usr/bin/env"]);
        #[cfg(windows)]
        let mut request = req(&["cmd.exe", "/D", "/S", "/C", "set \"INTELLIJ IDEA\""]);
        request.io_mode = io_mode;
        request.env.insert(
            "INTELLIJ IDEA".to_string(),
            Some("C:\\Program Files;Unicode 值".to_string()),
        );
        assert!(!request
            .command
            .iter()
            .any(|arg| arg.contains("INTELLIJ IDEA=")));
        let info = start_executor(&manager, request).unwrap();
        let output = wait_terminal(&manager, &info.executor_id);
        let stream = if io_mode.uses_pty() {
            OutputStream::Merged
        } else {
            OutputStream::Stdout
        };
        let text = merged_text(&output, stream);
        assert!(
            text.contains("C:\\Program Files;Unicode 值"),
            "io_mode={io_mode:?} status={:?} output={text:?}",
            output.status
        );
    }
}

#[test]
fn command_metadata_preserves_token_arguments() {
    let manager = ExecutorManager::default();
    let secret = "executor-log-secret-61b63a";
    let info = start_executor(&manager, req(&["sh", "-c", "exit 0", "--token", secret])).unwrap();
    assert_eq!(info.command.last().map(String::as_str), Some(secret));
    assert!(info.command.iter().any(|arg| arg == secret));
    let _ = wait_terminal(&manager, &info.executor_id);
    dispose_executor(&manager, &info.executor_id).unwrap();
}

#[test]
fn generic_start_is_atomically_rejected_after_id_reservation() {
    let manager = Arc::new(ExecutorManager::default());
    let prepared = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let worker_manager = manager.clone();
    let worker_prepared = prepared.clone();
    let worker_resume = resume.clone();
    let worker = std::thread::spawn(move || {
        let mut request = req(&["sh", "-c", "sleep 1"]);
        request.executor_id = Some("reserved-race".to_string());
        worker_prepared.wait();
        worker_resume.wait();
        start_executor(&worker_manager, request)
    });

    prepared.wait();
    assert_eq!(executor_registry_counts(&manager), (0, 0, 0));
    let reservation_id = ProcessOwnerId::generate();
    let reservation = reserve_executor_id(&manager, "reserved-race", &reservation_id).unwrap();
    resume.wait();
    assert!(worker.join().unwrap().is_err());
    assert_eq!(executor_registry_counts(&manager), (0, 1, 0));
    assert!(release_executor_reservation(&manager, &reservation));
    assert_eq!(executor_registry_counts(&manager), (0, 0, 0));
}

#[test]
fn matching_reserved_start_consumes_capability_and_missing_or_consumed_is_rejected() {
    let manager = ExecutorManager::default();
    let reservation_id = ProcessOwnerId::generate();
    let reservation = reserve_executor_id(&manager, "reserved-start", &reservation_id).unwrap();
    let mut generic = req(&["sh", "-c", "sleep 1"]);
    generic.executor_id = Some("reserved-start".to_string());
    assert!(start_executor(&manager, generic.clone()).is_err());

    let (_, receipt) = start_reserved_executor(&manager, generic.clone(), &reservation).unwrap();
    assert_eq!(receipt.process_id(), "reserved-start");
    assert_eq!(executor_registry_counts(&manager), (1, 0, 1));
    assert!(start_reserved_executor(&manager, generic, &reservation).is_err());
    terminate_reserved_executor(&manager, &receipt).unwrap();
    dispose_reserved_executor(&manager, &receipt).unwrap();
    assert_eq!(executor_registry_counts(&manager), (0, 0, 0));
}

#[test]
fn stale_capability_cannot_release_newer_same_id_reservation() {
    let manager = ExecutorManager::default();
    let old_id = ProcessOwnerId::generate();
    let old = reserve_executor_id(&manager, "same-id", &old_id).unwrap();
    assert!(release_executor_reservation(&manager, &old));
    let new_id = ProcessOwnerId::generate();
    let new = reserve_executor_id(&manager, "same-id", &new_id).unwrap();
    assert!(!release_executor_reservation(&manager, &old));
    assert_eq!(
        finalize_executor_reservation(&manager, "same-id", &old_id),
        crate::ownership::OwnedReleaseStatus::DifferentOwner
    );
    assert_eq!(executor_registry_counts(&manager), (0, 1, 0));
    assert!(release_executor_reservation(&manager, &new));
}

#[test]
fn committed_generic_executor_blocks_reservation_and_remains_untouched() {
    let manager = ExecutorManager::default();
    let mut request = req(&["sh", "-c", "sleep 30"]);
    request.executor_id = Some("generic-wins".to_string());
    let info = start_executor(&manager, request).unwrap();
    let reservation_id = ProcessOwnerId::generate();
    assert!(reserve_executor_id(&manager, "generic-wins", &reservation_id).is_err());
    assert!(get_executor_info(&manager, &info.executor_id).is_ok());
    terminate_executor(&manager, &info.executor_id).unwrap();
    dispose_executor(&manager, &info.executor_id).unwrap();
}

#[test]
fn cwd_is_applied() {
    let manager = ExecutorManager::default();
    let dir = std::env::temp_dir();
    let mut request = req(&["sh", "-c", "pwd"]);
    request.cwd = Some(dir.clone());
    let info = start_executor(&manager, request).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    let leaf = dir.file_name().unwrap().to_string_lossy().into_owned();
    assert!(merged_text(&out, OutputStream::Stdout).contains(&leaf));
}

#[test]
fn terminate_stops_long_running_process() {
    let manager = ExecutorManager::default();
    let info = start_executor(&manager, req(&["sh", "-c", "sleep 30"])).unwrap();
    // Running shortly after start.
    let running = read_executor_output(&manager, &info.executor_id, Some(0), None).unwrap();
    assert_eq!(running.status, ExecutorStatus::Running);
    terminate_executor(&manager, &info.executor_id).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert_eq!(out.status, ExecutorStatus::Cancelled);
}

#[cfg(unix)]
#[test]
fn reserved_pipe_interrupt_sends_catchable_sigint_without_marking_cancelled() {
    let manager = ExecutorManager::default();
    let executor_id = "owned-interrupt";
    let owner = ProcessOwnerId::generate();
    let reservation = reserve_executor_id(&manager, executor_id, &owner).unwrap();
    let mut request = req(&[
        "/bin/sh",
        "-c",
        "trap 'printf interrupted; exit 23' INT; printf ready; while :; do sleep 1; done",
    ]);
    request.executor_id = Some(executor_id.to_string());
    let (_, receipt) = start_reserved_executor(&manager, request, &reservation).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = read_reserved_executor_output(&manager, &receipt, Some(0), None).unwrap();
        if merged_text(&output, OutputStream::Stdout).contains("ready") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pipe process did not become ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let stale = crate::ownership::ProcessReceipt::for_test(executor_id, ProcessOwnerId::generate());
    assert!(interrupt_reserved_executor(&manager, &stale).is_err());
    interrupt_reserved_executor(&manager, &receipt).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let output = loop {
        let output = read_reserved_executor_output(&manager, &receipt, Some(0), None).unwrap();
        if output.status.is_terminal() {
            break output;
        }
        assert!(Instant::now() < deadline, "interrupted pipe did not finish");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(output.status, ExecutorStatus::Exited);
    assert_eq!(output.exit_code, Some(23));
    assert!(merged_text(&output, OutputStream::Stdout).contains("interrupted"));
    dispose_reserved_executor(&manager, &receipt).unwrap();
}

#[test]
fn timeout_reports_timed_out_and_stops_process() {
    let manager = ExecutorManager::default();
    // Very small timeout stresses the timeout/backend-registration ordering:
    // the timer starts only after the backend is registered.
    let mut request = req(&["sh", "-c", "sleep 30"]);
    request.timeout_ms = Some(1);
    let info = start_executor(&manager, request).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert_eq!(out.status, ExecutorStatus::TimedOut);
}

#[test]
fn multiple_executors_run_concurrently() {
    let manager = ExecutorManager::default();
    let a = start_executor(&manager, req(&["sh", "-c", "printf AAA"])).unwrap();
    let b = start_executor(&manager, req(&["sh", "-c", "printf BBB"])).unwrap();
    assert_ne!(a.executor_id, b.executor_id);
    let out_a = wait_terminal(&manager, &a.executor_id);
    let out_b = wait_terminal(&manager, &b.executor_id);
    assert!(merged_text(&out_a, OutputStream::Stdout).contains("AAA"));
    assert!(merged_text(&out_b, OutputStream::Stdout).contains("BBB"));
}

#[test]
fn pty_mode_captures_merged_output() {
    let manager = ExecutorManager::default();
    let mut request = req(&["sh", "-c", "printf pty-hello"]);
    request.io_mode = ExecutorIoMode::Pty;
    let info = start_executor(&manager, request).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert!(merged_text(&out, OutputStream::Merged).contains("pty-hello"));
}

#[test]
fn pty_request_normalizes_streaming() {
    // PTY mode forces both stdin and output streaming (Codex parity), even when
    // the caller left stream_stdin unset / stream_output false.
    let mut request = req(&["sh", "-c", "true"]);
    request.io_mode = ExecutorIoMode::Pty;
    request.stream_stdin = false;
    request.stream_output = false;
    request.normalize_io_mode();
    assert!(request.stream_stdin);
    assert!(request.stream_output);

    // Pipe mode is left untouched.
    let mut pipe = req(&["sh", "-c", "true"]);
    pipe.stream_stdin = false;
    pipe.normalize_io_mode();
    assert!(!pipe.stream_stdin);
}

#[test]
fn pty_stdin_is_writable_without_explicit_stream_stdin() {
    // The frontend demo starts a PTY without setting stream_stdin; the writer
    // must still be available because PTY spawn always keeps it.
    let manager = ExecutorManager::default();
    let mut request = req(&["sh", "-c", "read line; printf 'got:%s' \"$line\""]);
    request.io_mode = ExecutorIoMode::Pty;
    request.stream_stdin = false;
    let info = start_executor(&manager, request).unwrap();

    // Wait until it is actually running before writing.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let out = read_executor_output(&manager, &info.executor_id, Some(0), None).unwrap();
        if out.status == ExecutorStatus::Running || out.status.is_terminal() {
            break;
        }
        assert!(Instant::now() < deadline, "executor never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    write_executor_stdin(&manager, &info.executor_id, b"hi\n".to_vec(), true).unwrap();

    let out = wait_terminal(&manager, &info.executor_id);
    assert!(
        merged_text(&out, OutputStream::Merged).contains("got:hi"),
        "unexpected pty output: {:?}",
        merged_text(&out, OutputStream::Merged)
    );
}

fn wait_running(manager: &ExecutorManager, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let out = read_executor_output(manager, id, Some(0), None).unwrap();
        if out.status == ExecutorStatus::Running || out.status.is_terminal() {
            return;
        }
        assert!(Instant::now() < deadline, "executor never started");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn unread_pty_input_never_blocks_writes_or_termination() {
    // A raw-mode program that stops reading input fills the PTY input queue.
    // Writes are queued instead of blocking, a full queue is reported, and
    // termination is never stuck behind a pending write.
    let manager = ExecutorManager::default();
    let mut request = req(&["sh", "-c", "stty raw -echo; sleep 30"]);
    request.io_mode = ExecutorIoMode::Pty;
    let info = start_executor(&manager, request).unwrap();
    wait_running(&manager, &info.executor_id);

    let started = Instant::now();
    let mut rejected = false;
    for _ in 0..512 {
        if write_executor_stdin(&manager, &info.executor_id, vec![b'a'; 16 * 1024], false).is_err()
        {
            rejected = true;
            break;
        }
    }
    assert!(rejected, "the input queue must be bounded");
    assert!(started.elapsed() < Duration::from_secs(5), "writes blocked");

    let (done, finished) = std::sync::mpsc::channel();
    let terminator = manager.clone();
    let id = info.executor_id.clone();
    std::thread::spawn(move || {
        let _ = done.send(terminate_executor(&terminator, &id));
    });
    finished
        .recv_timeout(Duration::from_secs(5))
        .expect("terminate blocked behind a pending stdin write")
        .unwrap();
    assert_eq!(
        wait_terminal(&manager, &info.executor_id).status,
        ExecutorStatus::Cancelled
    );
    dispose_executor(&manager, &info.executor_id).unwrap();
}

#[test]
fn pty_answers_a_device_attributes_query() {
    // The program waits for its terminal's reply; the reader answers it as
    // input and keeps the query out of the output.
    let manager = ExecutorManager::default();
    let mut request = req(&[
        "sh",
        "-c",
        "stty raw -echo; printf '\\033[c'; head -c 7 | od -An -tx1",
    ]);
    request.io_mode = ExecutorIoMode::Pty;
    request.timeout_ms = Some(5_000);
    let info = start_executor(&manager, request).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert_eq!(out.status, ExecutorStatus::Exited);
    let text = merged_text(&out, OutputStream::Merged);
    // GNU and BSD `od` space their columns differently.
    let bytes = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(bytes.contains("1b 5b 3f 31 3b 32 63"), "{text:?}");
    assert!(!text.contains("\u{1b}[c"), "{text:?}");
}

#[test]
fn pipe_output_reads_in_slices_without_splitting_characters() {
    // 3000 three-byte characters written in one block: a reader decoding each
    // capped read on its own sees no replacement characters.
    let manager = ExecutorManager::default();
    let info = start_executor(
        &manager,
        req(&[
            "sh",
            "-c",
            "i=0; s=''; while [ $i -lt 3000 ]; do s=\"${s}中\"; i=$((i+1)); done; printf '%s' \"$s\"",
        ]),
    )
    .unwrap();
    wait_terminal(&manager, &info.executor_id);
    let mut offset = 0;
    let mut text = String::new();
    loop {
        let out = read_executor_output(&manager, &info.executor_id, Some(offset), Some(8 * 1024))
            .unwrap();
        if out.next_offset == offset {
            break;
        }
        for chunk in &out.chunks {
            text.push_str(&String::from_utf8_lossy(&chunk.data));
        }
        offset = out.next_offset;
    }
    assert_eq!(offset, 9000);
    assert_eq!(text, "中".repeat(3000));
}

#[test]
#[ignore = "reproduces §4.9 step 8, DaFilter holds an escape at end of file; fixed in E3"]
fn pty_output_ending_inside_an_escape_sequence_reaches_the_buffer() {
    // `DaFilter` holds `ESC [` and what follows until a final byte that never
    // comes: the bytes must still arrive once the PTY reaches end of file.
    let manager = ExecutorManager::default();
    let mut request = req(&["sh", "-c", "printf 'end\\033[ 99%%'"]);
    request.io_mode = ExecutorIoMode::Pty;
    request.timeout_ms = Some(5_000);
    let info = start_executor(&manager, request).unwrap();
    let out = wait_terminal(&manager, &info.executor_id);
    assert_eq!(out.status, ExecutorStatus::Exited);
    let bytes = out
        .chunks
        .iter()
        .flat_map(|chunk| chunk.data.clone())
        .collect::<Vec<_>>();
    assert!(
        bytes.ends_with(b"end\x1b[ 99%"),
        "{:?}",
        String::from_utf8_lossy(&bytes)
    );
}
