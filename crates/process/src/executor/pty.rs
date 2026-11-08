//! PTY execution backend (`portable-pty`): merged output, resize + stdin write,
//! and process-tree cleanup via the shared terminate-group helper. It spawns the
//! command directly under a fresh PTY and never touches the user-facing
//! interactive terminal.
//!
//! The reader answers terminal queries ([`DaFilter`]) and pushes whole UTF-8
//! scalars. It holds the output buffer and the input queue, never the session,
//! so releasing the backend closes the PTY and lets the reader end.

use std::collections::HashMap;
use std::io::{ErrorKind, Read};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::spawn_contained_thread;
use hanafubuki_agent_foundation::module_faults::FaultOwner;
use parking_lot::Mutex;

#[cfg(not(windows))]
use portable_pty::native_pty_system;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
// `PtySystem` must be in scope to call `.openpty()` on the concrete Windows
// `ConPtySystem`; on Unix the trait object provides the method directly.
#[cfg(windows)]
use portable_pty::PtySystem;

use super::backend_common::StdinWriter;
use super::model::{EnvironmentInheritance, ExecutorStartRequest, OutputStream};
use super::output::Utf8Carry;
use super::session::{ExecutorBackend, ExecutorSession};
#[cfg(windows)]
use super::windows_input::WindowsTtyInputNormalizer;
use crate::da_filter::DaFilter;

const READ_BUF: usize = 16 * 1024;
const POLL: Duration = Duration::from_millis(15);
const WOULD_BLOCK_SLEEP: Duration = Duration::from_millis(5);
const DRAIN_GRACE: Duration = Duration::from_millis(500);

struct PtyBackend {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    stdin: Arc<StdinWriter>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    #[cfg_attr(not(unix), allow(dead_code))]
    child_pid: Option<u32>,
    /// The ConPTY slave must outlive the process on Windows (Codex parity); dropping
    /// it early can leave the child with no attached console and stall output. On
    /// Unix the slave is dropped right after spawn as before.
    #[cfg(windows)]
    _slave: Mutex<Option<Box<dyn portable_pty::SlavePty + Send>>>,
    /// Windows Job Object (`KILL_ON_JOB_CLOSE`) covering the ConPTY child tree.
    #[cfg(windows)]
    job: Mutex<Option<crate::tree::ProcessJob>>,
    /// Normalizes newline/backspace bytes for Windows ConPTY stdin (Codex parity).
    #[cfg(windows)]
    input_normalizer: Mutex<WindowsTtyInputNormalizer>,
}

impl ExecutorBackend for PtyBackend {
    fn write_stdin(&self, bytes: Vec<u8>, close: bool) -> Result<(), String> {
        // Windows ConPTY expects CR for Enter and DEL for backspace; normalize the
        // incoming bytes first, queueing under the normalizer so concurrent
        // writes keep its state in order. Unix writes pass through unchanged.
        #[cfg(windows)]
        {
            let mut normalizer = self.input_normalizer.lock();
            let bytes = normalizer.normalize(&bytes);
            self.stdin.write(bytes, close)
        }
        #[cfg(not(windows))]
        self.stdin.write(bytes, close)
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .lock()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| err.to_string())
    }

    fn interrupt(&self) -> Result<(), String> {
        Err("explicit interrupt is only supported in pipe mode; write ETX to the PTY".to_string())
    }

    fn terminate(&self) -> Result<(), String> {
        terminate_pty(self)
    }

    #[cfg(unix)]
    fn process_group_leader(&self) -> Option<u32> {
        self.child_pid
    }
}

pub(crate) fn spawn(
    session: Arc<ExecutorSession>,
    request: &ExecutorStartRequest,
    inherited_environment: &HashMap<String, Option<String>>,
) -> Result<Box<dyn ExecutorBackend>, String> {
    let size = request.terminal_size();
    let (cols, rows) = (size.cols, size.rows);
    let executor_id = session.executor_id.clone();
    // On Windows use the Codex-derived custom ConPTY backend (proven stable) rather
    // than the default portable-pty system; Unix keeps the native system.
    #[cfg(windows)]
    let pty_system = crate::executor::win::ConPtySystem::default();
    #[cfg(not(windows))]
    let pty_system = native_pty_system();
    let backend_kind = if cfg!(windows) {
        "codex-conpty"
    } else {
        "portable-pty"
    };
    log::info!(
        "executor pty spawn start id={executor_id} backend={backend_kind} argv0={} cwd={:?} cols={cols} rows={rows}",
        request.command[0],
        request.cwd.as_deref()
    );
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| {
            log::error!("executor pty openpty failed id={executor_id}: {err}");
            err.to_string()
        })?;
    log::debug!("executor pty openpty ok id={executor_id}");

    let mut cmd = CommandBuilder::new(&request.command[0]);
    for arg in &request.command[1..] {
        cmd.arg(arg);
    }
    if let Some(cwd) = &request.cwd {
        cmd.cwd(cwd);
    }
    if request.environment_inheritance == EnvironmentInheritance::Clear {
        cmd.env_clear();
    } else {
        for (key, value) in inherited_environment {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            }
        }
    }
    for (key, value) in &request.env {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        }
    }

    let mut child = pair.slave.spawn_command(cmd).map_err(|err| {
        log::error!("executor pty spawn_command failed id={executor_id}: {err}");
        err.to_string()
    })?;
    log::debug!("executor pty spawn_command ok id={executor_id}");
    // Unix: the slave fd is no longer needed once the child owns it. Windows: keep
    // the ConPTY slave handle alive for the process lifetime (Codex parity).
    #[cfg(not(windows))]
    drop(pair.slave);

    let child_pid = child.process_id();
    log::info!(
        "executor pty child spawned id={executor_id} pid={:?}",
        child_pid
    );
    // On Windows, place the ConPTY child in a Job Object for tree cleanup.
    #[cfg(windows)]
    let job = match child_pid {
        Some(pid) => match crate::tree::ProcessJob::create_for(pid) {
            Ok(job) => {
                log::info!("executor pty job object created id={executor_id} pid={pid}");
                Some(job)
            }
            Err(err) => {
                log::warn!("executor pty job object failed id={executor_id} pid={pid}: {err}");
                None
            }
        },
        None => None,
    };
    let killer = child.clone_killer();
    let reader = pair.master.try_clone_reader().map_err(|err| {
        log::error!("executor pty try_clone_reader failed id={executor_id}: {err}");
        err.to_string()
    })?;
    log::debug!("executor pty reader cloned id={executor_id}");
    // PTY spawn always takes and keeps the writer (Codex parity); a failure here is
    // a spawn error rather than a silently-dropped writer.
    let writer = pair.master.take_writer().map_err(|err| {
        log::error!("executor pty take_writer failed id={executor_id}: {err}");
        err.to_string()
    })?;
    log::info!("executor pty writer acquired id={executor_id}");
    let stdin = match StdinWriter::spawn(&executor_id, writer) {
        Ok(stdin) => Arc::new(stdin),
        Err(error) => {
            let _ = child.kill();
            return Err(error);
        }
    };
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));

    let reader_output = session.output.clone();
    let reader_replies = stdin.clone();
    let reader_id = executor_id.clone();
    let reader_handle = spawn_contained_thread(
        FaultOwner::Core,
        format!("exec-pty-read-{executor_id}"),
        move || {
            log::debug!("executor pty reader started id={reader_id}");
            let mut reader = reader;
            let mut buf = [0u8; READ_BUF];
            let mut filtered = Vec::with_capacity(READ_BUF);
            let mut queries = DaFilter::new();
            let mut carry = Utf8Carry::default();
            let mut push = |bytes: &[u8]| {
                reader_output.lock().push(OutputStream::Merged, bytes)
            };
            let mut interrupted_retries: u64 = 0;
            let mut would_block_retries: u64 = 0;
            let exit_reason: &str = loop {
                match reader.read(&mut buf) {
                    Ok(0) => break "eof",
                    Ok(n) => {
                        filtered.clear();
                        queries.process(&buf[..n], &mut filtered, |reply| {
                            if let Err(error) = reader_replies.write(reply.to_vec(), false) {
                                log::debug!(
                                    "executor pty query reply dropped id={reader_id}: {error}"
                                );
                            }
                        });
                        carry.push(&filtered, &mut push);
                    }
                    // Transient conditions: keep reading instead of ending the loop.
                    Err(ref err) if err.kind() == ErrorKind::Interrupted => {
                        interrupted_retries += 1;
                        continue;
                    }
                    Err(ref err) if err.kind() == ErrorKind::WouldBlock => {
                        would_block_retries += 1;
                        thread::sleep(WOULD_BLOCK_SLEEP);
                        continue;
                    }
                    Err(err) => {
                        log::warn!("executor pty reader error id={reader_id}: {err}");
                        break "error";
                    }
                }
            };
            carry.finish(&mut push);
            log::debug!(
                "executor pty reader ended id={reader_id} reason={exit_reason} interrupted_retries={interrupted_retries} would_block_retries={would_block_retries}"
            );
        },
    )
    .map_err(|err| format!("spawn pty reader: {err}"))?;

    let waiter_session = session;
    let waiter_id = executor_id.clone();
    let master_for_waiter = master.clone();
    spawn_contained_thread(
        FaultOwner::Core,
        format!("exec-pty-wait-{executor_id}"),
        move || {
            log::debug!("executor pty waiter started id={waiter_id}");
            let code = match child.wait() {
                Ok(status) => Some(status.exit_code() as i32),
                Err(err) => {
                    log::warn!("executor pty wait failed id={waiter_id}: {err}");
                    waiter_session.finalize(None, Some(format!("wait failed: {err}")));
                    return;
                }
            };
            // Separate exit detection from output drain so late bytes are not lost:
            // give the reader a short grace to finish before releasing the master.
            let deadline = Instant::now() + DRAIN_GRACE;
            while Instant::now() < deadline && !reader_handle.is_finished() {
                thread::sleep(POLL);
            }
            // Keep the master alive until the reader is done, then release the PTY.
            drop(master_for_waiter);
            log::info!("executor pty waiter exit id={waiter_id} code={:?}", code);
            waiter_session.finalize(code, None);
        },
    )
    .map_err(|err| format!("spawn pty waiter: {err}"))?;

    Ok(Box::new(PtyBackend {
        master,
        stdin,
        killer: Mutex::new(killer),
        child_pid,
        #[cfg(windows)]
        _slave: Mutex::new(Some(pair.slave)),
        #[cfg(windows)]
        job: Mutex::new(job),
        #[cfg(windows)]
        input_normalizer: Mutex::new(WindowsTtyInputNormalizer::default()),
    }))
}

#[cfg(unix)]
fn terminate_pty(backend: &PtyBackend) -> Result<(), String> {
    log::info!(
        "executor pty terminate requested pid={:?}",
        backend.child_pid
    );
    if let Some(pid) = backend.child_pid {
        // The command is a session/process-group leader (portable-pty setsid);
        // signal the whole group.
        crate::tree::terminate_group_confirmed(pid)?;
    } else {
        backend
            .killer
            .lock()
            .kill()
            .map_err(|error| format!("terminate PTY process: {error}"))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate_pty(backend: &PtyBackend) -> Result<(), String> {
    log::info!(
        "executor pty terminate requested pid={:?}",
        backend.child_pid
    );
    // Drop the Job Object first to fire KILL_ON_JOB_CLOSE on the child tree, then
    // best-effort kill the direct ConPTY child.
    #[cfg(windows)]
    drop(backend.job.lock().take());
    if let Err(error) = backend.killer.lock().kill() {
        log::debug!("PTY direct-child kill after job teardown failed: {error}");
    }
    // Release the ConPTY slave once the child is being torn down.
    #[cfg(windows)]
    drop(backend._slave.lock().take());
    Ok(())
}
