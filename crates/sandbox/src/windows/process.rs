// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/windows-sandbox-rs/src/process.rs` (`create_process_as_user`:
// the job, the inherited handles, the desktop) and
// `src/proc_thread_attr.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the command inherits the helper's own
// standard handles, console and environment, and only them; the helper
// waits for it and returns its exit status; Ctrl+C is left to the command.

use std::ffi::c_void;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{
    SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, TRUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Console::{
    GetConsoleWindow, GetStdHandle, SetConsoleCtrlHandler, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use super::desktop::PrivateDesktop;
use super::token::last_error;

/// Starts `command_line` with `token` on `desktop` and waits for it.
///
/// The command starts in a job that kills what is left of it when the
/// helper exits, so it ends with the helper the `ExecutorManager` owns
/// (and its descendants with it). It inherits the helper's standard
/// handles and nothing else, its console (no window when there is none),
/// working directory and environment.
pub(crate) fn run(
    token: &OwnedHandle,
    command_line: &str,
    desktop: &PrivateDesktop,
) -> Result<u32, String> {
    let job = kill_on_close_job()?;
    let job_handle = job.as_raw_handle() as HANDLE;
    let standard = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(standard_handle);
    let mut inherited: Vec<HANDLE> = Vec::new();
    for handle in standard.into_iter().flatten() {
        if !inherited.contains(&handle) {
            // SAFETY: a handle this process holds.
            if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }
                == 0
            {
                return Err(last_error("SetHandleInformation"));
            }
            inherited.push(handle);
        }
    }
    let mut attributes = AttributeList::new(1 + u32::from(!inherited.is_empty()))?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST,
        std::ptr::addr_of!(job_handle).cast(),
        std::mem::size_of::<HANDLE>(),
    )?;
    if !inherited.is_empty() {
        attributes.set(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            inherited.as_ptr().cast(),
            std::mem::size_of_val(inherited.as_slice()),
        )?;
    }

    // SAFETY: zeroed is a valid STARTUPINFOEXW; its pointers are set below.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.lpDesktop = desktop.startup_name();
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    let [input, output, error] = standard.map(|handle| handle.unwrap_or(std::ptr::null_mut()));
    startup.StartupInfo.hStdInput = input;
    startup.StartupInfo.hStdOutput = output;
    startup.StartupInfo.hStdError = error;
    startup.lpAttributeList = attributes.as_ptr();

    // SAFETY: no pointer is read.
    let console = if unsafe { GetConsoleWindow() }.is_null() {
        CREATE_NO_WINDOW
    } else {
        0
    };
    // The command takes Ctrl+C as it likes; the helper waits to report it.
    // SAFETY: the handler is a plain function; it is not inherited.
    unsafe { SetConsoleCtrlHandler(Some(ignore_control), TRUE) };

    let mut command_line = wide(command_line);
    // SAFETY: zeroed is a valid PROCESS_INFORMATION.
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer outlives the call; `token` is a primary token
    // restricted from this process's own, which needs no privilege.
    let created = unsafe {
        CreateProcessAsUserW(
            token.as_raw_handle() as HANDLE,
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            BOOL::from(!inherited.is_empty()),
            CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | console,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::addr_of!(startup.StartupInfo),
            &mut information,
        )
    };
    if created == 0 {
        return Err(last_error("CreateProcessAsUserW"));
    }
    // SAFETY: both handles are new and this function's.
    let (process, _thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(information.hProcess as _),
            OwnedHandle::from_raw_handle(information.hThread as _),
        )
    };
    // SAFETY: a process handle with SYNCHRONIZE.
    if unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, INFINITE) } != WAIT_OBJECT_0
    {
        return Err(last_error("WaitForSingleObject"));
    }
    let mut status = 0u32;
    // SAFETY: a process handle with PROCESS_QUERY_INFORMATION.
    if unsafe { GetExitCodeProcess(process.as_raw_handle() as HANDLE, &mut status) } == 0 {
        return Err(last_error("GetExitCodeProcess"));
    }
    Ok(status)
}

unsafe extern "system" fn ignore_control(_kind: u32) -> BOOL {
    TRUE
}

/// The standard handle `which`, unless this process has none.
fn standard_handle(which: u32) -> Option<HANDLE> {
    // SAFETY: no pointer is read.
    let handle = unsafe { GetStdHandle(which) };
    (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(handle)
}

pub(crate) fn kill_on_close_job() -> Result<OwnedHandle, String> {
    // SAFETY: an unnamed job with the default security; owned below.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(last_error("CreateJobObjectW"));
    }
    // SAFETY: `job` is a new handle no one else owns.
    let job = unsafe { OwnedHandle::from_raw_handle(job as _) };
    // SAFETY: zeroed is a valid JOBOBJECT_EXTENDED_LIMIT_INFORMATION.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` is the structure the class names.
    let set = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(limits).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set == 0 {
        return Err(last_error("SetInformationJobObject"));
    }
    Ok(job)
}

/// A process's attribute list; the values it points to must outlive it.
struct AttributeList {
    buffer: Vec<u64>,
}

impl AttributeList {
    fn new(count: u32) -> Result<Self, String> {
        let mut size = 0usize;
        // SAFETY: a size query writes only `size`.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size) };
        let mut buffer = vec![0u64; size.div_ceil(8)];
        // SAFETY: the buffer holds `size` bytes.
        if unsafe {
            InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), count, 0, &mut size)
        } == 0
        {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }
        Ok(Self { buffer })
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buffer.as_mut_ptr().cast()
    }

    fn set(&mut self, attribute: u32, value: *const c_void, size: usize) -> Result<(), String> {
        // SAFETY: the list was initialized; the caller keeps `value` alive.
        let set = unsafe {
            UpdateProcThreadAttribute(
                self.as_ptr(),
                0,
                attribute as usize,
                value,
                size,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if set == 0 {
            return Err(last_error("UpdateProcThreadAttribute"));
        }
        Ok(())
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialized.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

/// `text` as a NUL-terminated UTF-16 string.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
