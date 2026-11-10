//! Pipe execution backend (`std::process`): separate stdout/stderr, streamed
//! into the session's tagged output buffer. No shell is used unless the caller's
//! argv provides one.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::spawn_contained_thread;
use hanafubuki_agent_foundation::module_faults::FaultOwner;
use parking_lot::Mutex;

use super::backend_common::StdinWriter;
use super::manager::ProcessSupervision;
use super::model::{EnvironmentInheritance, ExecutorStartRequest, OutputStream};
use super::output::{OutputBuffer, Utf8Carry};
use super::session::{ExecutorBackend, ExecutorSession};
use super::spawn_retry::{is_executable_busy, spawn_retrying_busy_executable};

const POLL: Duration = Duration::from_millis(15);
const READ_BUF: usize = 16 * 1024;
/// Bounded grace to let readers drain after the process exits.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// The three pipes of a process whose owner reads and writes them itself
/// ([`crate::executor::manager::start_reserved_pipe_executor`]): the
/// executor keeps no reader or writer on them, only the process.
pub struct ExecutorPipes {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    pub process_id: u32,
}

/// Whether the session's backend reads the process's output into its
/// buffer and queues its input, or hands the pipes to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PipeHandling {
    Buffered,
    HandedOut,
}

struct PipeBackend {
    child: Arc<Mutex<Child>>,
    stdin: Option<StdinWriter>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
    /// Windows Job Object (`KILL_ON_JOB_CLOSE`) covering the child tree; dropping
    /// it kills the whole tree.
    #[cfg(windows)]
    job: Mutex<Option<crate::tree::ProcessJob>>,
}

impl ExecutorBackend for PipeBackend {
    fn write_stdin(&self, bytes: Vec<u8>, close: bool) -> Result<(), String> {
        self.stdin
            .as_ref()
            .ok_or_else(|| {
                "stdin is not available (stream_stdin was not set, or its owner holds the pipe)"
                    .to_string()
            })?
            .write(bytes, close)
    }

    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), String> {
        Err("resize is only supported in pty mode".to_string())
    }

    fn interrupt(&self) -> Result<(), String> {
        interrupt_pipe(self)
    }

    fn terminate(&self) -> Result<(), String> {
        terminate_pipe(self)
    }

    #[cfg(unix)]
    fn process_group_leader(&self) -> Option<u32> {
        self.pid
    }
}

pub(crate) fn spawn(
    session: Arc<ExecutorSession>,
    request: &ExecutorStartRequest,
    supervision: ProcessSupervision,
    inherited_environment: &HashMap<String, Option<String>>,
    handling: PipeHandling,
) -> Result<(Box<dyn ExecutorBackend>, Option<ExecutorPipes>), String> {
    // Supervision selects a Windows Job Object policy. Elsewhere the child is
    // always given its own session below, and whether it outlives this process
    // depends only on whether anyone signals that group, so there is no
    // per-supervision spawn decision to make.
    #[cfg(not(windows))]
    let _ = supervision;

    let pipe_spawn_started = Instant::now();
    log::info!(
        "agent_diag stage=executor.pipe.prepare.begin executor_id={}",
        session.executor_id
    );
    let mut cmd = Command::new(&request.command[0]);
    cmd.args(&request.command[1..]);
    if let Some(cwd) = &request.cwd {
        // The request carries the native form because that is what is logged,
        // shown, and sent to a model. This is the Win32 boundary, so it is
        // where the verbatim prefix goes back on for a long path.
        cmd.current_dir(hanafubuki_agent_foundation::paths::system_form(cwd).as_ref());
    }
    if request.environment_inheritance == EnvironmentInheritance::Clear {
        cmd.env_clear();
    } else {
        apply_env(&mut cmd, inherited_environment);
    }
    apply_env(&mut cmd, &request.env);
    let handed_out = handling == PipeHandling::HandedOut;
    if request.stream_output || handed_out {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        // Output capture disabled: discard child output entirely.
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    cmd.stdin(if request.stream_stdin || handed_out {
        Stdio::piped()
    } else {
        Stdio::null()
    });

    // The packaged Tauri application is a Windows GUI-subsystem process and has
    // no console for background children to inherit. Without an explicit process
    // creation policy, short-lived console executables (Python, Git, Micromamba,
    // CLI helpers, and non-PTY tasks) can briefly allocate a visible console
    // window. Keep that platform policy at the Executor spawn boundary so every
    // pipe-mode caller receives it without duplicating Windows flags upstream.
    //
    // Do not apply this flag to the PTY backend: ConPTY supplies the child's
    // console semantics without a traditional host window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    // On Unix put the child in its own session/process group so termination can
    // signal the whole tree.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    let child_spawn_started = Instant::now();
    log::info!(
        "agent_diag stage=executor.pipe.child_spawn.begin executor_id={}",
        session.executor_id
    );
    let mut child =
        spawn_retrying_busy_executable(&session.executor_id, || cmd.spawn(), is_executable_busy)
            .map_err(|err| err.to_string())?;
    log::info!(
        "agent_diag stage=executor.pipe.child_spawn.end executor_id={} outcome=ok elapsed_ms={}",
        session.executor_id,
        child_spawn_started.elapsed().as_millis()
    );
    // On Windows, place the child in a Job Object so termination kills its tree.
    #[cfg(windows)]
    let job = match supervision {
        ProcessSupervision::Tree => match crate::tree::ProcessJob::create_for(child.id()) {
            Ok(job) => Some(job),
            Err(error) => {
                log::warn!(
                    "create Windows job object for executor {} failed; tree kill may not reach descendants: {error}",
                    session.executor_id
                );
                None
            }
        },
        ProcessSupervision::Independent => None,
    };
    let pid = Some(child.id());
    let (stdin, readers, pipes) = if handed_out {
        (None, Vec::new(), Some(hand_out(&mut child)?))
    } else {
        let stdin = match child.stdin.take() {
            Some(stdin) => match StdinWriter::spawn(&session.executor_id, Box::new(stdin)) {
                Ok(writer) => Some(writer),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            },
            None => None,
        };
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(spawn_reader(&session, out, OutputStream::Stdout));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(spawn_reader(&session, err, OutputStream::Stderr));
        }
        (stdin, readers, None)
    };
    let child = Arc::new(Mutex::new(child));

    let waiter_child = child.clone();
    let waiter_session = session.clone();
    spawn_contained_thread(
        FaultOwner::Core,
        format!("exec-wait-{}", waiter_session.executor_id),
        move || waiter(waiter_session, waiter_child, readers),
    )
    .map_err(|err| format!("spawn pipe waiter: {err}"))?;

    log::info!(
        "agent_diag stage=executor.pipe.prepare.end executor_id={} outcome=ok elapsed_ms={}",
        session.executor_id,
        pipe_spawn_started.elapsed().as_millis()
    );

    Ok((
        Box::new(PipeBackend {
            child,
            stdin,
            pid,
            #[cfg(windows)]
            job: Mutex::new(job),
        }),
        pipes,
    ))
}

/// The child's three pipes, for an owner that reads and writes them itself;
/// a child without them is killed.
fn hand_out(child: &mut Child) -> Result<ExecutorPipes, String> {
    match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
        (Some(stdin), Some(stdout), Some(stderr)) => Ok(ExecutorPipes {
            stdin,
            stdout,
            stderr,
            process_id: child.id(),
        }),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            Err("the process's pipes were not opened".to_string())
        }
    }
}

fn apply_env(cmd: &mut Command, env: &HashMap<String, Option<String>>) {
    for (key, value) in env {
        match value {
            Some(value) => {
                cmd.env(key, value);
            }
            None => {
                cmd.env_remove(key);
            }
        }
    }
}

/// A reader holds only the output buffer, never the session: a descendant that
/// keeps the pipe open must not keep the session and its backend alive.
fn spawn_reader<R: Read + Send + 'static>(
    session: &ExecutorSession,
    mut reader: R,
    stream: OutputStream,
) -> thread::JoinHandle<()> {
    let output: Arc<Mutex<OutputBuffer>> = session.output.clone();
    spawn_contained_thread(
        FaultOwner::Core,
        format!("exec-read-{}", session.executor_id),
        move || {
            let mut buf = [0u8; READ_BUF];
            let mut carry = Utf8Carry::default();
            let mut push = |bytes: &[u8]| output.lock().push(stream, bytes);
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => carry.push(&buf[..n], &mut push),
                    Err(_) => break,
                }
            }
            carry.finish(&mut push);
        },
    )
    .expect("spawn pipe reader thread")
}

fn waiter(
    session: Arc<ExecutorSession>,
    child: Arc<Mutex<Child>>,
    readers: Vec<thread::JoinHandle<()>>,
) {
    let waiter_started = Instant::now();
    log::info!(
        "agent_diag stage=executor.pipe.waiter.begin executor_id={}",
        session.executor_id
    );
    let status = loop {
        let status = child.lock().try_wait();
        match status {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(err) => {
                session.finalize(None, Some(format!("wait failed: {err}")));
                return;
            }
        }
        thread::sleep(POLL);
    };

    // Let readers drain remaining output for a bounded period.
    let deadline = Instant::now() + DRAIN_GRACE;
    while Instant::now() < deadline && readers.iter().any(|h| !h.is_finished()) {
        thread::sleep(POLL);
    }

    session.finalize_exited(status);
    log::info!(
        "agent_diag stage=executor.pipe.waiter.end executor_id={} outcome=terminal elapsed_ms={}",
        session.executor_id,
        waiter_started.elapsed().as_millis()
    );
}

#[cfg(unix)]
fn interrupt_pipe(backend: &PipeBackend) -> Result<(), String> {
    let Some(pid) = backend.pid else {
        return Err("pipe process id is unavailable".to_string());
    };
    let group = pid as libc::pid_t;
    let result = unsafe { libc::kill(-group, libc::SIGINT) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!(
            "send SIGINT to pipe process group {group}: {error}"
        ))
    }
}

#[cfg(not(unix))]
fn interrupt_pipe(backend: &PipeBackend) -> Result<(), String> {
    // CREATE_NO_WINDOW pipe children have no console to receive a soft Ctrl+C.
    // Match Codex's Windows pipe behavior by stopping the owned Job/direct
    // process. This deliberately does not set ExecutorSession::terminate_requested;
    // the waiter remains the authority for the observed terminal outcome.
    terminate_pipe(backend)
}

#[cfg(unix)]
fn terminate_pipe(backend: &PipeBackend) -> Result<(), String> {
    match backend.pid {
        Some(pid) => {
            crate::tree::terminate_group_confirmed(pid)?;
            backend
                .child
                .lock()
                .wait()
                .map_err(|error| format!("wait for terminated pipe process {pid}: {error}"))?;
        }
        None => {
            let mut child = backend.child.lock();
            if let Err(kill_error) = child.kill() {
                if child
                    .try_wait()
                    .map_err(|error| error.to_string())?
                    .is_none()
                {
                    return Err(format!("terminate pipe process: {kill_error}"));
                }
            }
            child
                .wait()
                .map_err(|error| format!("wait for terminated pipe process: {error}"))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate_pipe(backend: &PipeBackend) -> Result<(), String> {
    // Drop the Job Object first to fire KILL_ON_JOB_CLOSE on the whole tree,
    // then reap the direct child.
    #[cfg(windows)]
    drop(backend.job.lock().take());
    let mut child = backend.child.lock();
    if let Err(kill_error) = child.kill() {
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err(format!("terminate pipe process: {kill_error}"));
        }
    }
    child
        .wait()
        .map_err(|error| format!("wait for terminated pipe process: {error}"))?;
    Ok(())
}
