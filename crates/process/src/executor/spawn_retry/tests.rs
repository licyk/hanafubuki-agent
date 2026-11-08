use std::io;
use std::time::Duration;

use super::{is_executable_busy, retry_busy};

const NO_WAIT: [Duration; 3] = [Duration::ZERO; 3];

#[cfg(unix)]
fn busy_error() -> io::Error {
    io::Error::from_raw_os_error(libc::ETXTBSY)
}

/// Runs `retry_busy` over `outcomes` in order and returns its result and the
/// number of attempts it made.
fn run(outcomes: Vec<Result<&'static str, io::Error>>) -> (Result<&'static str, io::Error>, usize) {
    let mut outcomes = outcomes.into_iter();
    let mut attempts = 0;
    let result = retry_busy(
        "test",
        &NO_WAIT,
        &mut || {
            attempts += 1;
            outcomes.next().expect("no attempt past the last outcome")
        },
        &is_executable_busy,
    );
    (result, attempts)
}

#[test]
fn a_successful_spawn_is_not_repeated() {
    let (result, attempts) = run(vec![Ok("child")]);
    assert_eq!(result.unwrap(), "child");
    assert_eq!(attempts, 1);
}

#[test]
fn an_error_other_than_a_busy_executable_is_returned_at_once() {
    let (result, attempts) = run(vec![Err(io::Error::from(io::ErrorKind::NotFound))]);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    assert_eq!(attempts, 1);
    #[cfg(unix)]
    assert!(!is_executable_busy(&io::Error::from_raw_os_error(
        libc::ENOENT
    )));
}

#[cfg(unix)]
#[test]
fn a_busy_executable_is_spawned_once_it_is_released() {
    let (result, attempts) = run(vec![Err(busy_error()), Err(busy_error()), Ok("child")]);
    assert_eq!(result.unwrap(), "child");
    assert_eq!(attempts, 3);
}

#[cfg(unix)]
#[test]
fn a_busy_executable_fails_with_its_error_once_the_schedule_is_spent() {
    let outcomes = (0..=NO_WAIT.len()).map(|_| Err(busy_error())).collect();
    let (result, attempts) = run(outcomes);
    assert!(is_executable_busy(&result.unwrap_err()));
    assert_eq!(attempts, NO_WAIT.len() + 1);
}

#[cfg(unix)]
mod forked_sibling {
    use std::fs;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::executor::manager::run_and_wait;
    use crate::executor::model::{EnvironmentInheritance, ExecutorIoMode, ExecutorStartRequest};
    use crate::executor::ExecutorManager;

    use super::is_executable_busy;

    /// How long the forked sibling keeps the inherited descriptor before it
    /// execs: well inside the retry schedule's 511 ms.
    const HOLD_MICROS: libc::c_uint = 100_000;

    fn test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "hanafubuki-spawn-retry-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Recreates the race deterministically: a sibling thread forks while the
    /// script is open for writing, and its child holds the inherited
    /// descriptor before it execs. The script is closed and made executable
    /// first, as a publisher does, and spawning it directly then fails with
    /// `ETXTBSY`; the Executor's pipe spawn waits the window out.
    ///
    /// The PTY backend is not covered: `portable-pty` closes every inherited
    /// descriptor in the child before exec, including the one `std` reports
    /// an exec failure through, so a busy executable there is a child that
    /// aborts before exec rather than a spawn error to retry.
    #[test]
    fn pipe_spawn_waits_out_a_forked_sibling() {
        let dir = test_dir("pipe");
        let script = dir.join("published");
        let mut file = fs::File::create(&script).unwrap();
        file.write_all(b"#!/bin/sh\necho published\n").unwrap();

        let (mut forked, forked_signal) = std::io::pipe().unwrap();
        let sibling = thread::spawn(move || {
            let signal = forked_signal.as_raw_fd();
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "exit 0"]);
            // SAFETY: the closure calls only async-signal-safe functions.
            unsafe {
                command.pre_exec(move || {
                    libc::write(signal, b"f".as_ptr().cast(), 1);
                    libc::usleep(HOLD_MICROS);
                    Ok(())
                });
            }
            let status = command.status();
            drop(forked_signal);
            status
        });
        let mut forked_byte = [0_u8; 1];
        forked.read_exact(&mut forked_byte).unwrap();
        drop(file);
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let direct = Command::new(&script)
            .spawn()
            .expect_err("the forked sibling still holds the script open for writing");
        assert!(is_executable_busy(&direct), "unexpected error: {direct}");

        let manager = ExecutorManager::default();
        let request = ExecutorStartRequest {
            executor_id: None,
            command: vec![script.to_str().unwrap().to_string()],
            cwd: None,
            environment_inheritance: EnvironmentInheritance::Inherit,
            env: std::collections::HashMap::new(),
            io_mode: ExecutorIoMode::Pipe,
            stream_stdin: false,
            stream_output: true,
            cols: None,
            rows: None,
            timeout_ms: Some(10_000),
            output_bytes_cap: None,
        };
        let outcome = run_and_wait(&manager, request, Duration::from_secs(12)).unwrap();
        assert_eq!(outcome.exit_code, Some(0), "output: {}", outcome.output);
        assert!(outcome.output.contains("published"), "{}", outcome.output);
        assert!(sibling.join().unwrap().unwrap().success());
        let _ = fs::remove_dir_all(dir);
    }
}
