//! Shared cross-platform process-tree lifecycle infrastructure.
//!
//! This is neutral backend infrastructure, not part of any one subsystem.
//! On Unix, [`terminate_group`] is the single graceful process-tree teardown used
//! by both the user-facing interactive terminal in the application crate and
//! this crate's command executor ([`crate::executor`]). On Windows,
//! [`ProcessJob`] owns a Job Object configured with `KILL_ON_JOB_CLOSE` for the
//! same two consumers. Neither owner depends on the other's module.
//!
//! Unix implementation notes:
//! portable-pty runs `setsid()` in spawned children, so a PTY shell/command is a
//! session/process-group leader and its pid doubles as the process-group id.

#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
const POLL_INTERVAL: Duration = Duration::from_millis(15);
/// Grace period after a graceful terminate before force-killing the tree.
#[cfg(unix)]
const TERMINATE_GRACE: Duration = Duration::from_millis(300);

/// Terminate a Unix process group by pid/pgid using the shared graceful sequence:
/// `SIGTERM` to `-pgid`, a short grace period, then `SIGKILL` to `-pgid`. The
/// caller-provided completion signal may end the grace loop early; confirmed
/// executor disposal uses [`terminate_group_confirmed`] instead.
#[cfg(unix)]
pub fn terminate_group(pgid: u32, mut is_done: impl FnMut() -> bool) -> Result<(), String> {
    if is_done() {
        return Ok(());
    }
    let group = pgid as libc::pid_t;
    // Graceful terminate of the whole process group.
    signal_process_group(group, libc::SIGTERM)?;

    let deadline = Instant::now() + TERMINATE_GRACE;
    loop {
        if is_done() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Force-kill anything still alive in the group.
            signal_process_group(group, libc::SIGKILL)?;
            return Ok(());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Stricter executor cleanup: leader exit is not enough while descendants keep
/// the original process group alive.
#[cfg(unix)]
pub fn terminate_group_confirmed(pgid: u32) -> Result<(), String> {
    let group = pgid as libc::pid_t;
    if !process_group_exists(group)? {
        return Ok(());
    }
    signal_process_group(group, libc::SIGTERM)?;
    let deadline = Instant::now() + TERMINATE_GRACE;
    loop {
        if !process_group_exists(group)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            signal_process_group(group, libc::SIGKILL)?;
            return Ok(());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(unix)]
fn signal_process_group(group: libc::pid_t, signal: libc::c_int) -> Result<(), String> {
    let result = unsafe { libc::kill(-group, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(()),
        // XNU's killpg skips zombies and reports EPERM when nothing else is
        // left, as when a group leader has exited but is not yet reaped.
        #[cfg(target_os = "macos")]
        Some(libc::EPERM) => Ok(()),
        _ => Err(format!(
            "send signal {signal} to process group {group}: {error}"
        )),
    }
}

#[cfg(unix)]
fn process_group_exists(group: libc::pid_t) -> Result<bool, String> {
    let result = unsafe { libc::kill(-group, 0) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(format!("inspect process group {group}: {error}")),
    }
}

#[cfg(all(test, unix))]
mod tests;

/// Windows Job Object with `KILL_ON_JOB_CLOSE` for managed child process trees.
///
/// Holding this handle beside a spawned terminal or executor process lets
/// Windows kill all descendants before ConPTY or pipe resources are dropped.
#[cfg(windows)]
pub struct ProcessJob {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for ProcessJob {}
#[cfg(windows)]
unsafe impl Sync for ProcessJob {}

#[cfg(windows)]
impl ProcessJob {
    pub fn create_for(pid: u32) -> std::io::Result<Self> {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::Foundation::{CloseHandle, FALSE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() || job == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }

            let process = OpenProcess(PROCESS_TERMINATE | PROCESS_SET_QUOTA, FALSE, pid);
            if process.is_null() {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }

            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }

            Ok(Self { handle: job })
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};

        if !self.handle.is_null() && self.handle != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}
