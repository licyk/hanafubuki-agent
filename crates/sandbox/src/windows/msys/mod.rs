//! Experimental session-scoped MSYS IPC ACL compatibility.
//!
//! The restricting token never includes the reusable sandbox account SID.
//! MSYS's GENERIC_ALL account ACEs use this command's fresh logon SID instead.
//! Interception is confined to the loaded runtime; installed files are untouched.
mod image;
mod memory;
mod shim;

use std::collections::HashMap;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::Foundation::{
    CloseHandle, DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, EXCEPTION_BREAKPOINT, HANDLE,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    ContinueDebugEvent, GetThreadContext, SetThreadContext, WaitForDebugEvent, CONTEXT,
    CONTEXT_CONTROL_AMD64, CREATE_PROCESS_DEBUG_EVENT, DEBUG_EVENT, EXCEPTION_DEBUG_EVENT,
    EXIT_PROCESS_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, INFINITE, THREAD_GET_CONTEXT, THREAD_SET_CONTEXT,
};

use super::token::{current_logon_sid, current_user_sid, last_error};
use image::Image;

struct Process {
    handle: HANDLE,
    initial_breakpoint: bool,
    pending: Option<(Image, u8)>,
}

pub(super) fn wait(root: u32) -> Result<u32, String> {
    let account = current_user_sid()?;
    let logon = current_logon_sid()?;
    let mut processes = HashMap::<u32, Process>::new();
    loop {
        // SAFETY: the OS initializes this output structure on success.
        let mut event: DEBUG_EVENT = unsafe { std::mem::zeroed() };
        if unsafe { WaitForDebugEvent(&mut event, INFINITE) } == 0 {
            return Err(last_error("wait for sandbox debug event"));
        }
        let mut status = DBG_CONTINUE;
        let mut exit = None;
        // SAFETY: the event code selects the initialized member of the union.
        unsafe {
            match event.dwDebugEventCode {
                CREATE_PROCESS_DEBUG_EVENT => {
                    let info = event.u.CreateProcessInfo;
                    close_file(info.hFile);
                    // Windows closes these event handles after EXIT_PROCESS is continued.
                    processes.insert(
                        event.dwProcessId,
                        Process {
                            handle: info.hProcess,
                            initial_breakpoint: false,
                            pending: None,
                        },
                    );
                }
                LOAD_DLL_DEBUG_EVENT => {
                    let info = event.u.LoadDll;
                    close_file(info.hFile);
                    if let Some(process) = processes.get_mut(&event.dwProcessId) {
                        if let Some(image) = Image::msys(process.handle, info.lpBaseOfDll as usize)?
                        {
                            let original = memory::read(process.handle, image.entry, 1)?[0];
                            memory::write(process.handle, image.entry, &[0xcc])?;
                            process.pending = Some((image, original));
                        }
                    }
                }
                EXCEPTION_DEBUG_EVENT => {
                    status = DBG_EXCEPTION_NOT_HANDLED;
                    let exception = event.u.Exception.ExceptionRecord;
                    if exception.ExceptionCode == EXCEPTION_BREAKPOINT {
                        if let Some(process) = processes.get_mut(&event.dwProcessId) {
                            let address = exception.ExceptionAddress as usize;
                            if process
                                .pending
                                .as_ref()
                                .is_some_and(|(image, _)| image.entry == address)
                            {
                                if let Some((image, original)) = process.pending.take() {
                                    memory::write(process.handle, address, &[original])?;
                                    shim::install(process.handle, &image, &account, &logon)?;
                                    rewind_thread(event.dwThreadId, address)?;
                                    status = DBG_CONTINUE;
                                }
                            } else if !process.initial_breakpoint {
                                process.initial_breakpoint = true;
                                status = DBG_CONTINUE;
                            }
                        }
                    }
                }
                EXIT_PROCESS_DEBUG_EVENT => {
                    if event.dwProcessId == root {
                        exit = Some(event.u.ExitProcess.dwExitCode);
                    }
                    processes.remove(&event.dwProcessId);
                }
                _ => {}
            }
        }
        if unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, status) } == 0 {
            return Err(last_error("continue sandbox debug event"));
        }
        if let Some(code) = exit {
            return Ok(code);
        }
    }
}

fn rewind_thread(id: u32, address: usize) -> Result<(), String> {
    // SAFETY: OpenThread validates the event's thread ID; we own its returned handle.
    let thread = unsafe { OpenThread(THREAD_GET_CONTEXT | THREAD_SET_CONTEXT, 0, id) };
    if thread.is_null() {
        return Err(last_error("open MSYS initialization thread"));
    }
    let thread = unsafe { OwnedHandle::from_raw_handle(thread as _) };
    let mut context: CONTEXT = unsafe { std::mem::zeroed() };
    context.ContextFlags = CONTEXT_CONTROL_AMD64;
    if unsafe { GetThreadContext(thread.as_raw_handle() as _, &mut context) } == 0 {
        return Err(last_error("read MSYS initialization context"));
    }
    context.Rip = address as u64;
    if unsafe { SetThreadContext(thread.as_raw_handle() as _, &context) } == 0 {
        return Err(last_error("restore MSYS initialization context"));
    }
    Ok(())
}

unsafe fn close_file(handle: HANDLE) {
    if !handle.is_null() {
        CloseHandle(handle);
    }
}
