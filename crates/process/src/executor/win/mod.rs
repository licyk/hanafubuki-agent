#![allow(clippy::unwrap_used)]

//! Windows ConPTY backend for the executor, ported from Codex
//! (`codex-rs/utils/pty/src/win/`), which copied it from
//! https://github.com/wezterm/wezterm (MIT license).
//!
//! Codex uses this custom ConPTY `PtySystem` instead of
//! `portable_pty::native_pty_system()` on Windows; the executor PTY backend does
//! the same so its Windows behavior matches Codex's proven implementation.
//!
//! Local modifications (from Codex):
//! - Fix WezTerm/Codex bug #13945 in the Windows kill path: `TerminateProcess`
//!   returns nonzero on success, so only `0` is treated as an error.
//! - Tree termination uses the backend's own Job Object rather than a job
//!   attached at creation, so `ProcThreadAttributeList` sets only the
//!   pseudoconsole.
//! - The process and pseudoconsole handles sit behind `parking_lot` locks,
//!   which do not poison (AGENTS.md, "Locks").

use filedescriptor::OwnedHandle;
use parking_lot::Mutex;
use portable_pty::Child;
use portable_pty::ChildKiller;
use portable_pty::ExitStatus;
use std::io::Error as IoError;
use std::io::Result as IoResult;
use std::os::windows::io::AsRawHandle;
use winapi::shared::minwindef::DWORD;
use winapi::um::minwinbase::STILL_ACTIVE;
use winapi::um::processthreadsapi::*;
use winapi::um::synchapi::WaitForSingleObject;
use winapi::um::winbase::INFINITE;

pub(crate) mod conpty;
mod procthreadattr;
mod psuedocon;

pub use conpty::ConPtySystem;

#[derive(Debug)]
pub struct WinChild {
    proc: Mutex<OwnedHandle>,
}

impl WinChild {
    fn is_complete(&mut self) -> IoResult<Option<ExitStatus>> {
        let mut status: DWORD = 0;
        let proc = self.proc.lock().try_clone().unwrap();
        let res = unsafe { GetExitCodeProcess(proc.as_raw_handle() as _, &mut status) };
        if res != 0 {
            if status == STILL_ACTIVE {
                Ok(None)
            } else {
                Ok(Some(ExitStatus::with_exit_code(status)))
            }
        } else {
            Ok(None)
        }
    }

    fn do_kill(&mut self) -> IoResult<()> {
        let proc = self.proc.lock().try_clone().unwrap();
        let res = unsafe { TerminateProcess(proc.as_raw_handle() as _, 1) };
        // Codex bug #13945: Win32 returns nonzero on success, so only `0` is an error.
        if res == 0 {
            Err(IoError::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl ChildKiller for WinChild {
    fn kill(&mut self) -> IoResult<()> {
        if let Err(error) = self.do_kill() {
            log::warn!(
                "TerminateProcess failed for a Windows child while reporting success: {error}"
            );
        }
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        let proc = self.proc.lock().try_clone().unwrap();
        Box::new(WinChildKiller { proc })
    }
}

#[derive(Debug)]
pub struct WinChildKiller {
    proc: OwnedHandle,
}

impl ChildKiller for WinChildKiller {
    fn kill(&mut self) -> IoResult<()> {
        let res = unsafe { TerminateProcess(self.proc.as_raw_handle() as _, 1) };
        // Codex bug #13945: Win32 returns nonzero on success, so only `0` is an error.
        if res == 0 {
            Err(IoError::last_os_error())
        } else {
            Ok(())
        }
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        let proc = self.proc.try_clone().unwrap();
        Box::new(WinChildKiller { proc })
    }
}

impl Child for WinChild {
    fn try_wait(&mut self) -> IoResult<Option<ExitStatus>> {
        self.is_complete()
    }

    fn wait(&mut self) -> IoResult<ExitStatus> {
        if let Ok(Some(status)) = self.try_wait() {
            return Ok(status);
        }
        let proc = self.proc.lock().try_clone().unwrap();
        unsafe {
            WaitForSingleObject(proc.as_raw_handle() as _, INFINITE);
        }
        let mut status: DWORD = 0;
        let res = unsafe { GetExitCodeProcess(proc.as_raw_handle() as _, &mut status) };
        if res != 0 {
            Ok(ExitStatus::with_exit_code(status))
        } else {
            Err(IoError::last_os_error())
        }
    }

    fn process_id(&self) -> Option<u32> {
        let res = unsafe { GetProcessId(self.proc.lock().as_raw_handle() as _) };
        if res == 0 {
            None
        } else {
            Some(res)
        }
    }

    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        let proc = self.proc.lock();
        Some(proc.as_raw_handle())
    }
}
