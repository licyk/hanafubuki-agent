// Derived from Codex windows-sandbox-rs/framed_io.rs and stdio_bridge.rs
// (57a808a392, Apache-2.0). Three owner/account-only pipes carry raw stdio.

use std::fs::{File, OpenOptions};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr::null_mut;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Console::{
    SetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, SetNamedPipeHandleState, PIPE_NOWAIT,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

use super::super::{process::wide, token::last_error};
use super::{credentials, security};

pub(super) struct Pipes {
    pub name: String,
    pub input: File,
    pub output: File,
    pub error: File,
}

impl Pipes {
    pub fn create(account: &str) -> Result<Self, String> {
        let owner = security::current_user_sid()?;
        let sid = security::account_sid(account)?;
        let sid = security::sid_string(sid.as_ptr())?;
        let name = format!(
            r"\\.\pipe\hana-sbx-{:032x}",
            u128::from_le_bytes(credentials::random()?)
        );
        Ok(Self {
            input: create(&format!("{name}-in"), PIPE_ACCESS_DUPLEX, &owner, &sid)?,
            output: create(&format!("{name}-out"), PIPE_ACCESS_DUPLEX, &owner, &sid)?,
            error: create(&format!("{name}-err"), PIPE_ACCESS_DUPLEX, &owner, &sid)?,
            name,
        })
    }

    pub fn connect(&self, process: HANDLE) -> Result<(), String> {
        for file in [&self.input, &self.output, &self.error] {
            loop {
                // Nonblocking connect lets a runner that fails at startup be reaped.
                let connected =
                    unsafe { ConnectNamedPipe(file.as_raw_handle() as HANDLE, null_mut()) };
                let error = unsafe { GetLastError() };
                if connected != 0 || error == ERROR_PIPE_CONNECTED {
                    break;
                }
                if error != ERROR_PIPE_LISTENING {
                    return Err(last_error("connect sandbox stdio"));
                }
                if unsafe { WaitForSingleObject(process, 25) } != WAIT_TIMEOUT {
                    return Err("the sandbox runner exited before connecting its IO".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let mode = PIPE_WAIT;
            if unsafe {
                SetNamedPipeHandleState(
                    file.as_raw_handle() as HANDLE,
                    &mode,
                    null_mut(),
                    null_mut(),
                )
            } == 0
            {
                return Err(last_error("set sandbox pipe mode"));
            }
        }
        Ok(())
    }
}

fn create(name: &str, access: u32, owner: &str, account: &str) -> Result<File, String> {
    let sddl = wide(&format!("D:P(A;;GA;;;{owner})(A;;GRGW;;;{account})"));
    let mut descriptor = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error("sandbox pipe DACL"));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let name = wide(name);
    // SAFETY: descriptor and name remain live through pipe creation.
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            access | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            &attributes,
        )
    };
    unsafe { LocalFree(descriptor) };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(last_error("create sandbox stdio pipe"));
    }
    Ok(unsafe { File::from_raw_handle(pipe as _) })
}

pub(super) fn attach(name: &str) -> Result<[File; 3], String> {
    let input = OpenOptions::new()
        .read(true)
        .open(format!("{name}-in"))
        .map_err(|e| e.to_string())?;
    let output = OpenOptions::new()
        .write(true)
        .open(format!("{name}-out"))
        .map_err(|e| e.to_string())?;
    let error = OpenOptions::new()
        .write(true)
        .open(format!("{name}-err"))
        .map_err(|e| e.to_string())?;
    let files = [input, output, error];
    for (file, kind) in files
        .iter()
        .zip([STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE])
    {
        if unsafe { SetStdHandle(kind, file.as_raw_handle() as HANDLE) } == 0 {
            return Err(last_error("attach sandbox IO"));
        }
    }
    Ok(files)
}
