//! White-box tests for confirmed Executor disposal and ownership retention.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::disposal::{
    dispose_executor_confirmed, dispose_executor_confirmed_with_timeout,
    ConfirmedExecutorDisposalError, ConfirmedExecutorDisposalOutcome,
};
use super::start::apply_child_process_environment;
use super::*;
use crate::executor::model::ExecutorStartRequest;
use crate::executor::session::{ExecutorBackend, ExecutorSession};
#[cfg(unix)]
use crate::executor::EnvironmentInheritance;
use crate::executor::OutputStream;
use crate::ownership::ProcessOwnerId;
use crate::terminal_size::{TerminalResize, TerminalSize};

struct FailingTerminationBackend;

impl ExecutorBackend for FailingTerminationBackend {
    fn write_stdin(&self, _bytes: Vec<u8>, _close: bool) -> Result<(), String> {
        Ok(())
    }

    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), String> {
        Ok(())
    }

    fn interrupt(&self) -> Result<(), String> {
        Ok(())
    }

    fn terminate(&self) -> Result<(), String> {
        Err("injected process-tree termination failure".to_string())
    }
}

struct NonTerminatingBackend;

impl ExecutorBackend for NonTerminatingBackend {
    fn write_stdin(&self, _bytes: Vec<u8>, _close: bool) -> Result<(), String> {
        Ok(())
    }

    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), String> {
        Ok(())
    }

    fn interrupt(&self) -> Result<(), String> {
        Ok(())
    }

    fn terminate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Records every resize the session forwards to the PTY.
#[derive(Clone, Default)]
struct RecordingResizeBackend {
    resizes: Arc<Mutex<Vec<(u16, u16)>>>,
}

impl ExecutorBackend for RecordingResizeBackend {
    fn write_stdin(&self, _bytes: Vec<u8>, _close: bool) -> Result<(), String> {
        Ok(())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.resizes.lock().unwrap().push((cols, rows));
        Ok(())
    }

    fn interrupt(&self) -> Result<(), String> {
        Ok(())
    }

    fn terminate(&self) -> Result<(), String> {
        Ok(())
    }
}

fn request(command: Vec<String>) -> ExecutorStartRequest {
    ExecutorStartRequest {
        executor_id: None,
        command,
        cwd: None,
        environment_inheritance: super::super::model::EnvironmentInheritance::Inherit,
        env: HashMap::new(),
        io_mode: super::super::model::ExecutorIoMode::Pipe,
        stream_stdin: false,
        stream_output: true,
        cols: None,
        rows: None,
        timeout_ms: None,
        output_bytes_cap: None,
    }
}

fn insert_fake_session(
    manager: &ExecutorManager,
    executor_id: &str,
    backend: Box<dyn ExecutorBackend>,
) -> Arc<ExecutorSession> {
    let session = Arc::new(ExecutorSession::new(
        executor_id.to_string(),
        super::super::model::ExecutorIoMode::Pipe,
        vec!["fake".to_string()],
        1024,
        None,
        None,
    ));
    session.set_backend(backend);
    session.set_running();
    manager
        .registry
        .lock()
        .sessions
        .insert(executor_id.to_string(), session.clone());
    session
}

#[test]
fn child_process_environment_wins_for_pipe_pty_and_cleared_inheritance() {
    let environment = ChildProcessEnvironment::default();
    environment
        .replace(HashMap::from([
            ("HTTP_PROXY".into(), Some("http://global:8080".into())),
            ("HTTPS_PROXY".into(), None),
        ]))
        .unwrap();
    let manager = ExecutorManager::new(environment);
    let mut pipe = request(vec!["ignored".into()]);
    pipe.env
        .insert("HTTP_PROXY".into(), Some("http://instance:9000".into()));
    pipe.env
        .insert("HTTPS_PROXY".into(), Some("http://instance:9000".into()));
    apply_child_process_environment(&manager, &mut pipe);
    assert_eq!(
        pipe.env.get("HTTP_PROXY"),
        Some(&Some("http://global:8080".into()))
    );
    assert_eq!(pipe.env.get("HTTPS_PROXY"), Some(&None));

    let mut pty = request(vec!["ignored".into()]);
    pty.io_mode = super::super::model::ExecutorIoMode::Pty;
    pty.environment_inheritance = super::super::model::EnvironmentInheritance::Clear;
    apply_child_process_environment(&manager, &mut pty);
    assert_eq!(
        pty.env.get("HTTP_PROXY"),
        Some(&Some("http://global:8080".into()))
    );
    assert_eq!(pty.env.get("HTTPS_PROXY"), Some(&None));
}

#[test]
fn independent_start_rejects_pty() {
    let manager = ExecutorManager::default();
    let mut pty = request(vec!["ignored".into()]);
    pty.io_mode = super::super::model::ExecutorIoMode::Pty;
    assert!(start_independent_executor(&manager, pty)
        .unwrap_err()
        .contains("pipe mode only"));
}

#[cfg(unix)]
#[test]
fn child_process_environment_sets_and_removes_variables_in_real_pipe_child() {
    fn run_env(manager: &ExecutorManager, request_env: HashMap<String, Option<String>>) -> String {
        let mut request = request(vec!["/usr/bin/env".into()]);
        request.env = request_env;
        let info = start_executor(manager, request).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !get_executor_info(manager, &info.executor_id)
            .unwrap()
            .status
            .is_terminal()
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let output = read_executor_output(manager, &info.executor_id, Some(0), None).unwrap();
        String::from_utf8(
            output
                .chunks
                .into_iter()
                .flat_map(|chunk| chunk.data)
                .collect(),
        )
        .unwrap()
    }

    let environment = ChildProcessEnvironment::default();
    environment
        .replace(HashMap::from([
            ("HTTP_PROXY".into(), Some("http://global:8080".into())),
            ("HTTPS_PROXY".into(), Some("http://global:8080".into())),
        ]))
        .unwrap();
    let manager = ExecutorManager::new(environment.clone());
    let output = run_env(
        &manager,
        HashMap::from([("HTTP_PROXY".into(), Some("http://caller:9000".into()))]),
    );
    assert!(output.contains("HTTP_PROXY=http://global:8080"));
    assert!(output.contains("HTTPS_PROXY=http://global:8080"));

    environment
        .replace(HashMap::from([
            ("HTTP_PROXY".into(), None),
            ("HTTPS_PROXY".into(), None),
        ]))
        .unwrap();
    let output = run_env(
        &manager,
        HashMap::from([
            ("HTTP_PROXY".into(), Some("http://caller:9000".into())),
            ("HTTPS_PROXY".into(), Some("http://caller:9000".into())),
        ]),
    );
    assert!(!output.lines().any(|line| line.starts_with("HTTP_PROXY=")));
    assert!(!output.lines().any(|line| line.starts_with("HTTPS_PROXY=")));
}

#[cfg(unix)]
#[test]
fn inherited_repair_precedes_caller_and_final_environment() {
    fn run_env(
        environment: ChildProcessEnvironment,
        inheritance: EnvironmentInheritance,
        request_env: HashMap<String, Option<String>>,
    ) -> String {
        let manager = ExecutorManager::new(environment);
        let mut request = request(vec!["/usr/bin/env".into()]);
        request.environment_inheritance = inheritance;
        request.env = request_env;
        let info = start_executor(&manager, request).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !get_executor_info(&manager, &info.executor_id)
            .unwrap()
            .status
            .is_terminal()
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let output = read_executor_output(&manager, &info.executor_id, Some(0), None).unwrap();
        String::from_utf8(
            output
                .chunks
                .into_iter()
                .flat_map(|chunk| chunk.data)
                .collect(),
        )
        .unwrap()
    }

    let environment = ChildProcessEnvironment::with_inherited_overlay(HashMap::from([
        ("HANAFUBUKI_INHERITED_ORDER".into(), Some("base".into())),
        ("HANAFUBUKI_INHERITED_ONLY".into(), Some("base".into())),
    ]))
    .unwrap();
    environment
        .replace(HashMap::from([(
            "HANAFUBUKI_INHERITED_ORDER".into(),
            Some("final".into()),
        )]))
        .unwrap();
    let output = run_env(
        environment,
        EnvironmentInheritance::Inherit,
        HashMap::from([("HANAFUBUKI_INHERITED_ORDER".into(), Some("caller".into()))]),
    );
    assert!(output.contains("HANAFUBUKI_INHERITED_ORDER=final"));
    assert!(output.contains("HANAFUBUKI_INHERITED_ONLY=base"));

    let environment = ChildProcessEnvironment::with_inherited_overlay(HashMap::from([(
        "HANAFUBUKI_INHERITED_ONLY".into(),
        Some("base".into()),
    )]))
    .unwrap();
    let output = run_env(environment, EnvironmentInheritance::Clear, HashMap::new());
    assert!(!output.contains("HANAFUBUKI_INHERITED_ONLY="));
}

#[test]
fn confirmed_disposal_removes_an_already_exited_executor() {
    let manager = ExecutorManager::default();
    let executable = std::env::current_exe().unwrap();
    let info = start_executor(
        &manager,
        request(vec![
            executable.to_string_lossy().into_owned(),
            "--list".to_string(),
        ]),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !get_executor_info(&manager, &info.executor_id)
        .unwrap()
        .status
        .is_terminal()
    {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        dispose_executor_confirmed(&manager, &info.executor_id).unwrap(),
        ConfirmedExecutorDisposalOutcome::Disposed
    );
    assert!(get_executor_info(&manager, &info.executor_id).is_err());
}

#[test]
fn termination_error_and_timeout_retain_session_for_retry() {
    let manager = ExecutorManager::default();
    let failed = insert_fake_session(
        &manager,
        "termination-error",
        Box::new(FailingTerminationBackend),
    );
    let error = dispose_executor_confirmed_with_timeout(
        &manager,
        "termination-error",
        Duration::from_millis(5),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfirmedExecutorDisposalError::Termination { .. }
    ));
    assert!(get_executor_info(&manager, "termination-error").is_ok());
    failed.finalize(None, None);
    assert_eq!(
        dispose_executor_confirmed(&manager, "termination-error").unwrap(),
        ConfirmedExecutorDisposalOutcome::Disposed
    );

    insert_fake_session(
        &manager,
        "termination-timeout",
        Box::new(NonTerminatingBackend),
    );
    let error = dispose_executor_confirmed_with_timeout(
        &manager,
        "termination-timeout",
        Duration::from_millis(5),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfirmedExecutorDisposalError::Timeout { .. }
    ));
    assert!(get_executor_info(&manager, "termination-timeout").is_ok());
}

#[test]
fn reserved_cleanup_failure_retains_the_exact_owner_receipt_for_retry() {
    let manager = ExecutorManager::default();
    let executor_id = "owned-termination-error";
    let failed = insert_fake_session(&manager, executor_id, Box::new(FailingTerminationBackend));
    let owner = ProcessOwnerId::generate();
    let receipt = {
        let mut registry = manager.registry.lock();
        let reservation = registry.ownership.reserve(executor_id, &owner).unwrap();
        assert!(registry.ownership.begin_spawn(&reservation));
        registry.ownership.commit(&reservation).unwrap()
    };

    let error = dispose_reserved_executor(&manager, &receipt).unwrap_err();
    assert!(error.contains("injected process-tree termination failure"));
    assert!(manager.registry.lock().ownership.authorize(&receipt));
    assert_eq!(executor_registry_counts(&manager), (1, 0, 1));

    failed.finalize(None, None);
    dispose_reserved_executor(&manager, &receipt).unwrap();
    assert_eq!(executor_registry_counts(&manager), (0, 0, 0));
}

#[test]
fn already_absent_confirmed_disposal_is_idempotent() {
    let manager = ExecutorManager::default();
    assert_eq!(
        dispose_executor_confirmed(&manager, "missing").unwrap(),
        ConfirmedExecutorDisposalOutcome::AlreadyAbsent
    );
}

#[cfg(unix)]
#[test]
fn domain_owned_write_and_resize_require_the_committed_receipt() {
    let manager = ExecutorManager::default();
    let executor_id = "owned-interactive";
    let owner = ProcessOwnerId::generate();
    let reservation = reserve_executor_id(&manager, executor_id, &owner).unwrap();
    let request = ExecutorStartRequest {
        executor_id: Some(executor_id.to_string()),
        command: vec!["/bin/sh".to_string()],
        cwd: Some(std::env::temp_dir()),
        environment_inheritance: super::super::model::EnvironmentInheritance::Inherit,
        env: HashMap::new(),
        io_mode: super::super::model::ExecutorIoMode::Pty,
        stream_stdin: true,
        stream_output: true,
        cols: Some(80),
        rows: Some(24),
        timeout_ms: None,
        output_bytes_cap: None,
    };
    let (_, receipt) = start_reserved_executor(&manager, request, &reservation).unwrap();

    let write_error =
        write_executor_stdin(&manager, executor_id, b"exit\n".to_vec(), false).unwrap_err();
    assert!(write_error.contains("domain-owned"));
    let resize_error = resize_executor_pty(&manager, executor_id, 100, 30).unwrap_err();
    assert!(resize_error.contains("domain-owned"));
    assert!(terminate_executor(&manager, executor_id)
        .unwrap_err()
        .contains("domain-owned"));
    assert!(dispose_executor(&manager, executor_id)
        .unwrap_err()
        .contains("domain-owned"));

    resize_reserved_executor_pty(&manager, &receipt, 100, 30).unwrap();
    write_reserved_executor_stdin(&manager, &receipt, b"exit\n".to_vec(), true).unwrap();
    dispose_reserved_executor(&manager, &receipt).unwrap();
}

#[cfg(unix)]
#[test]
fn a_handed_out_pipe_executor_gives_its_owner_the_pipes_and_its_exit_status() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;

    let manager = ExecutorManager::default();
    let owner = ProcessOwnerId::generate();
    let reservation = reserve_executor_id(&manager, "handed-out", &owner).unwrap();
    let mut request = request(vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "read line; echo \"got $line\"; echo err >&2; exec sleep 30".to_string(),
    ]);
    request.executor_id = Some("handed-out".to_string());
    let (_, receipt, mut pipes) =
        start_reserved_pipe_executor(&manager, request, &reservation).unwrap();

    writeln!(pipes.stdin, "hello").unwrap();
    let mut line = String::new();
    BufReader::new(&mut pipes.stdout)
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "got hello\n");
    let mut error = String::new();
    BufReader::new(&mut pipes.stderr)
        .read_line(&mut error)
        .unwrap();
    assert_eq!(error, "err\n");
    // The session reads and writes nothing of its own.
    let output = read_reserved_executor_output(&manager, &receipt, None, None).unwrap();
    assert!(output.chunks.is_empty());
    assert!(write_reserved_executor_stdin(&manager, &receipt, b"x".to_vec(), false).is_err());
    assert_eq!(
        wait_reserved_executor(&manager, &receipt, Duration::ZERO).unwrap(),
        None
    );

    terminate_reserved_executor(&manager, &receipt).unwrap();
    let status = wait_reserved_executor(&manager, &receipt, Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
    assert!(pipes.process_id > 0);
    dispose_reserved_executor(&manager, &receipt).unwrap();
    assert!(!reserved_executor_exists(&manager, &receipt));
}

const PROCESS_TREE_HELPER_TEST: &str = "executor::manager::tests::process_tree_helper";

#[test]
fn process_tree_helper() {
    let Ok(mode) = std::env::var("HANAFUBUKI_CONFIRMED_DISPOSAL_HELPER") else {
        return;
    };
    if mode == "child" {
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
        }
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }
    if mode == "parent" {
        let executable = std::env::current_exe().unwrap();
        #[allow(
            clippy::zombie_processes,
            reason = "the helper runs until the test terminates its whole tree"
        )]
        let child = Command::new(executable)
            .args(["--exact", PROCESS_TREE_HELPER_TEST, "--nocapture"])
            .env("HANAFUBUKI_CONFIRMED_DISPOSAL_HELPER", "child")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        println!("DESCENDANT_PID={}", child.id());
        std::io::stdout().flush().unwrap();
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }
}

#[test]
fn confirmed_disposal_terminates_running_pipe_process_tree() {
    let manager = ExecutorManager::default();
    let executable = std::env::current_exe().unwrap();
    let mut request = request(vec![
        executable.to_string_lossy().into_owned(),
        "--exact".to_string(),
        PROCESS_TREE_HELPER_TEST.to_string(),
        "--nocapture".to_string(),
    ]);
    request.env.insert(
        "HANAFUBUKI_CONFIRMED_DISPOSAL_HELPER".to_string(),
        Some("parent".to_string()),
    );
    let info = start_executor(&manager, request).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let _descendant = loop {
        let output = read_executor_output(&manager, &info.executor_id, Some(0), None).unwrap();
        let bytes: Vec<_> = output
            .chunks
            .into_iter()
            .flat_map(|chunk| chunk.data)
            .collect();
        let text = String::from_utf8_lossy(&bytes);
        if let Some(pid) = text
            .lines()
            .find_map(|line| line.trim().strip_prefix("DESCENDANT_PID="))
            .and_then(|value| value.parse::<u32>().ok())
        {
            break pid;
        }
        assert!(
            Instant::now() < deadline,
            "helper did not report descendant pid"
        );
        thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(
        dispose_executor_confirmed(&manager, &info.executor_id).unwrap(),
        ConfirmedExecutorDisposalOutcome::Disposed
    );
    assert!(get_executor_info(&manager, &info.executor_id).is_err());

    #[cfg(unix)]
    {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let result = unsafe { libc::kill(_descendant as libc::pid_t, 0) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "descendant process {_descendant} survived confirmed disposal"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn pty_resize_records_each_change_and_skips_the_current_size() {
    let backend = RecordingResizeBackend::default();
    let session = ExecutorSession::new(
        "pty-resize".to_string(),
        super::super::model::ExecutorIoMode::Pty,
        vec!["fake".to_string()],
        1024,
        Some(TerminalSize::DEFAULT),
        None,
    );
    session.set_backend(Box::new(backend.clone()));
    session
        .output
        .lock()
        .push(OutputStream::Merged, b"0123456789");

    let wide = TerminalSize::new(132, 40);
    session.resize(TerminalSize::DEFAULT).unwrap();
    session.resize(wide).unwrap();
    session.resize(wide).unwrap();

    // Only the real change reaches the PTY, so ConPTY repaints once.
    assert_eq!(*backend.resizes.lock().unwrap(), vec![(132, 40)]);
    let output = session.output.lock();
    assert_eq!(
        output.terminal_sizes(0, 10),
        (Some(TerminalSize::DEFAULT), Vec::new())
    );
    assert_eq!(output.terminal_sizes(10, 10), (Some(wide), Vec::new()));
    assert_eq!(
        output.terminal_sizes(0, 11).1,
        vec![TerminalResize {
            offset: 10,
            size: wide
        }]
    );
}

#[test]
fn read_reports_the_size_history_of_the_returned_range() {
    let manager = ExecutorManager::default();
    let backend = RecordingResizeBackend::default();
    let session = Arc::new(ExecutorSession::new(
        "pty-read".to_string(),
        super::super::model::ExecutorIoMode::Pty,
        vec!["fake".to_string()],
        1024,
        Some(TerminalSize::DEFAULT),
        None,
    ));
    session.set_backend(Box::new(backend));
    session.set_running();
    manager
        .registry
        .lock()
        .sessions
        .insert("pty-read".to_string(), session.clone());

    session.output.lock().push(OutputStream::Merged, b"narrow");
    resize_executor_pty(&manager, "pty-read", 120, 30).unwrap();
    session.output.lock().push(OutputStream::Merged, b"wide");

    let output = read_executor_output(&manager, "pty-read", None, None).unwrap();
    assert_eq!(output.terminal_size, Some(TerminalSize::DEFAULT));
    assert_eq!(
        output.resizes,
        vec![TerminalResize {
            offset: 6,
            size: TerminalSize::new(120, 30)
        }]
    );
    let tail = read_executor_output(&manager, "pty-read", Some(6), None).unwrap();
    assert_eq!(tail.terminal_size, Some(TerminalSize::new(120, 30)));
    assert!(tail.resizes.is_empty());
}
