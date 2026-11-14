// Derived from Codex windows-sandbox-rs/setup.rs (57a808a392, Apache-2.0).
// UAC runs setup only; ordinary commands never run as administrator.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};

use super::super::{process::wide, token::last_error};
use super::credentials::Credentials;
use super::{firewall, security};

pub(crate) const SETUP_ARG: &str = "--hanafubuki-agent-sandbox-setup";

pub(super) struct Installation {
    pub root: ResolvedPath,
    pub credentials: Credentials,
}

pub(super) fn directory() -> Result<ResolvedPath, String> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is missing")?;
    ResolvedPath::resolve(PathBuf::from(base).join("hanafubuki-agent/sandbox"))
        .map_err(|error| format!("sandbox directory: {error}"))
}

pub(super) fn load(root: ResolvedPath) -> Result<Installation, String> {
    let file = root.system().join("secrets/accounts.json");
    let bytes = std::fs::read(file)
        .map_err(|_| "Windows sandbox is not set up; run /setup-default-sandbox")?;
    let credentials: Credentials = serde_json::from_slice(&bytes)
        .map_err(|_| "Windows sandbox credentials cannot be read; the file was preserved")?;
    for account in [&credentials.online, &credentials.offline] {
        if !account.name.starts_with("hana-")
            || account.name.len() > 20
            || !account
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(
                "invalid sandbox account identity; the credentials file was preserved".into(),
            );
        }
    }
    Ok(Installation { root, credentials })
}

pub(crate) fn ready() -> Result<(), String> {
    let install = load(directory()?)?;
    if !install.root.system().join("ready").is_file() {
        return Err("Windows sandbox setup is incomplete; run /setup-default-sandbox".into());
    }
    install.credentials.online.verify()?;
    install.credentials.offline.verify()?;
    firewall::verify(&install.credentials.keys())
}

pub(crate) fn setup() -> Result<(), String> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is missing")?;
    let base = ResolvedPath::resolve(base).map_err(|e| e.to_string())?;
    let app = ResolvedPath::resolve_child(&base, std::ffi::OsStr::new("hanafubuki-agent"))
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(app.system()).map_err(|e| e.to_string())?;
    let root = ResolvedPath::resolve_child(&app, std::ffi::OsStr::new("sandbox"))
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root.system()).map_err(|e| e.to_string())?;
    let owner = security::current_user_sid()?;
    let secret_dir = root.system().join("secrets");
    std::fs::create_dir_all(&secret_dir).map_err(|e| e.to_string())?;
    security::protect_secrets(&secret_dir, &owner)?;
    let accounts = secret_dir.join("accounts.json");
    if !accounts.exists() {
        let credentials = Credentials::create()?;
        let bytes =
            serde_json::to_vec(&credentials).map_err(|_| "cannot encode sandbox credentials")?;
        std::fs::write(&accounts, bytes).map_err(|e| e.to_string())?;
    }
    let install = load(root)?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(install.root.system().join("ready"));
    let _ = std::fs::remove_file(install.root.system().join("setup-error"));
    elevate(
        &executable,
        &[
            SETUP_ARG.to_string(),
            install.root.native_text().to_string(),
        ],
    )?;
    ready()
}

pub(crate) fn elevated_setup(root: PathBuf) -> Result<(), String> {
    let root = ResolvedPath::resolve(root).map_err(|e| e.to_string())?;
    let result = provision(&root);
    if let Err(error) = &result {
        let _ = std::fs::write(root.system().join("setup-error"), error);
    }
    result
}

fn provision(root: &ResolvedPath) -> Result<(), String> {
    let install = load(root.clone())?;
    install.credentials.online.provision()?;
    install.credentials.offline.provision()?;
    security::enable_standard_account(&install.credentials.online.name)?;
    security::enable_standard_account(&install.credentials.offline.name)?;
    install.credentials.online.verify()?;
    install.credentials.offline.verify()?;
    firewall::install(
        &install.credentials.offline.name,
        &install.credentials.keys(),
    )?;
    std::fs::write(root.system().join("ready"), b"1\n").map_err(|e| e.to_string())
}

fn elevate(program: &std::path::Path, args: &[String]) -> Result<(), String> {
    let program: Vec<u16> = program.as_os_str().encode_wide().chain(Some(0)).collect();
    // The existing argv quoting also handles arguments, after dropping its dummy executable.
    let mut argv = vec!["setup".to_string()];
    argv.extend_from_slice(args);
    let command = super::super::plan::command_line(&argv)?;
    let parameters = wide(
        command
            .strip_prefix("\"setup\" ")
            .ok_or("setup arguments")?,
    );
    let verb = wide("runas");
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = program.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = 0;
    // SAFETY: all pointers live through synchronous activation; retain the exact
    // process handle until setup terminates, including when UAC uses another admin.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(last_error(
            "Windows sandbox setup was not started (UAC may have been cancelled)",
        ));
    }
    if info.hProcess.is_null() {
        return Err("setup returned no process handle".into());
    }
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess as _) };
    if unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, INFINITE) } != WAIT_OBJECT_0
    {
        return Err(last_error("wait for sandbox setup"));
    }
    let mut status = 0;
    if unsafe { GetExitCodeProcess(process.as_raw_handle() as HANDLE, &mut status) } == 0 {
        return Err(last_error("sandbox setup status"));
    }
    if status != 0 {
        let detail = directory()
            .ok()
            .and_then(|root| std::fs::read_to_string(root.system().join("setup-error")).ok())
            .unwrap_or_else(|| format!("exit code {status}"));
        return Err(format!("Windows sandbox setup failed: {detail}"));
    }
    Ok(())
}
