// Derived from Codex windows-sandbox-rs/elevated_impl.rs and process.rs
// (57a808a392, Apache-2.0). Log on as the sandbox account, never elevate a command.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;

use hanafubuki_agent_foundation::background_worker::spawn_contained_thread;
use hanafubuki_agent_foundation::module_faults::FaultOwner;
use hanafubuki_agent_foundation::paths::ResolvedPath;
use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
use windows_sys::Win32::System::Threading::{
    CreateProcessWithLogonW, GetExitCodeProcess, ResumeThread, TerminateProcess,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, INFINITE,
    LOGON_WITH_PROFILE, PROCESS_INFORMATION, STARTUPINFOW,
};

use super::super::{acl, plan, process, token::last_error};
use super::{pipes::Pipes, security, setup};
use crate::helper::HelperRequest;

pub(crate) const RUNNER_ARG: &str = "--hanafubuki-agent-sandbox-runner";

pub(crate) fn run(request: HelperRequest) -> Result<u32, String> {
    setup::ready()?;
    let install = setup::load(setup::directory()?)?;
    let account = if request.network_access {
        &install.credentials.online
    } else {
        &install.credentials.offline
    };
    let sid = security::account_sid(&account.name)?;
    let runner = ResolvedPath::resolve(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    acl::ensure_allow_read_ace(runner.system(), &sid)?;
    let cwd = ResolvedPath::resolve(std::env::current_dir().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    // The primary identity needs ordinary access too; the restricted token's
    // capability check still confines writes to this command's current roots.
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let profile = ResolvedPath::resolve(profile).map_err(|e| e.to_string())?;
        acl::ensure_allow_read_ace(profile.system(), &sid)?;
    }
    acl::ensure_allow_read_ace(cwd.system(), &sid)?;
    for root in crate::roots::resolve_roots(&request.writable_roots) {
        acl::ensure_allow_write_ace(root.root.system(), &sid)?;
    }
    super::super::prepare(&request)?;
    let pipes = Pipes::create(&account.name)?;
    let mut argv = vec![
        runner.native_text().to_string(),
        RUNNER_ARG.into(),
        pipes.name.clone(),
    ];
    argv.extend(request.to_args().ok_or("sandbox arguments must be UTF-8")?);
    let mut command = process::wide(&plan::command_line(&argv)?);
    let application = process::wide(runner.native_text());
    let cwd = process::wide(cwd.native_text());
    let username = process::wide(&account.name);
    let domain = process::wide(".");
    let password = account.password()?;
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let job = process::kill_on_close_job()?;
    // SAFETY: all strings live through the call. The child is suspended until
    // its exact process handle belongs to our kill-on-close job.
    let started = unsafe {
        CreateProcessWithLogonW(
            username.as_ptr(),
            domain.as_ptr(),
            password.0.as_ptr(),
            LOGON_WITH_PROFILE,
            application.as_ptr(),
            command.as_mut_ptr(),
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            null(),
            cwd.as_ptr(),
            &startup,
            &mut info,
        )
    };
    if started == 0 {
        return Err(last_error(
            "start sandbox account runner; run /setup-default-sandbox to repair",
        ));
    }
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess as _) };
    let thread = unsafe { OwnedHandle::from_raw_handle(info.hThread as _) };
    let child = Child {
        process,
        job: Some(job),
    };
    if unsafe {
        AssignProcessToJobObject(
            child.job.as_ref().ok_or("sandbox job")?.as_raw_handle() as HANDLE,
            child.handle(),
        )
    } == 0
    {
        return Err(last_error("assign sandbox runner to its job"));
    }
    if unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) } == u32::MAX {
        return Err(last_error("resume sandbox runner"));
    }
    pipes.connect(child.handle())?;
    bridge(child, pipes)
}

struct Child {
    process: OwnedHandle,
    job: Option<OwnedHandle>,
}

impl Child {
    fn handle(&self) -> HANDLE {
        self.process.as_raw_handle() as HANDLE
    }

    fn wait(&mut self) -> Result<u32, String> {
        if unsafe { WaitForSingleObject(self.handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(last_error("wait for sandbox runner"));
        }
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.handle(), &mut code) } == 0 {
            return Err(last_error("sandbox runner status"));
        }
        // End descendants before draining pipes they may have inherited.
        self.job.take();
        Ok(code)
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.job.take();
        // Also covers failure before the suspended process was assigned to the job.
        unsafe {
            TerminateProcess(self.handle(), 126);
            WaitForSingleObject(self.handle(), INFINITE);
        }
    }
}

fn bridge(mut child: Child, pipes: Pipes) -> Result<u32, String> {
    let Pipes {
        mut input,
        mut output,
        mut error,
        ..
    } = pipes;
    let _input = spawn_contained_thread(FaultOwner::Core, "sandbox-stdin", move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut input);
    })
    .map_err(|e| e.to_string())?;
    let output = spawn_contained_thread(FaultOwner::Core, "sandbox-stdout", move || {
        std::io::copy(&mut output, &mut std::io::stdout().lock())
    })
    .map_err(|e| e.to_string())?;
    let error = spawn_contained_thread(FaultOwner::Core, "sandbox-stderr", move || {
        std::io::copy(&mut error, &mut std::io::stderr().lock())
    })
    .map_err(|e| e.to_string())?;
    let code = child.wait()?;
    output
        .join()
        .map_err(|_| "sandbox stdout worker failed")?
        .map_err(|e| e.to_string())?;
    error
        .join()
        .map_err(|_| "sandbox stderr worker failed")?
        .map_err(|e| e.to_string())?;
    // stdin may be waiting for the enclosing ExecutorManager; the helper exits
    // immediately after returning the child's status, ending that reader too.
    Ok(code)
}
