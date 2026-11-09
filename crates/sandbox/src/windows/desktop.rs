// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/windows-sandbox-rs/src/desktop.rs` (`PrivateDesktop::create`,
// `grant_desktop_access`) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: one desktop per command, which the helper
// holds until the command ends, so no shared cache; named after the helper
// and the time rather than a random number; on this process's window
// station rather than always `WinSta0`.

use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::{
    SetEntriesInAclW, SetSecurityInfo, EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_WINDOW_OBJECT,
    TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{ACL, DACL_SECURITY_INFORMATION};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, GetProcessWindowStation, GetUserObjectInformationW,
    DESKTOP_CREATEMENU, DESKTOP_CREATEWINDOW, DESKTOP_DELETE, DESKTOP_ENUMERATE,
    DESKTOP_HOOKCONTROL, DESKTOP_JOURNALPLAYBACK, DESKTOP_JOURNALRECORD, DESKTOP_READOBJECTS,
    DESKTOP_READ_CONTROL, DESKTOP_SWITCHDESKTOP, DESKTOP_WRITEOBJECTS, DESKTOP_WRITE_DAC,
    DESKTOP_WRITE_OWNER, HDESK, UOI_NAME,
};

use super::token::last_error;

const DESKTOP_ALL_ACCESS: u32 = DESKTOP_READOBJECTS
    | DESKTOP_CREATEWINDOW
    | DESKTOP_CREATEMENU
    | DESKTOP_HOOKCONTROL
    | DESKTOP_JOURNALRECORD
    | DESKTOP_JOURNALPLAYBACK
    | DESKTOP_ENUMERATE
    | DESKTOP_WRITEOBJECTS
    | DESKTOP_SWITCHDESKTOP
    | DESKTOP_DELETE
    | DESKTOP_READ_CONTROL
    | DESKTOP_WRITE_DAC
    | DESKTOP_WRITE_OWNER;

/// A desktop of the command's own, so its windows, hooks and input never
/// reach the user's (a restricted token alone shares the user's desktop).
/// Some programs (PowerShell) also fail to start under a restricted token
/// that names no desktop.
pub(crate) struct PrivateDesktop {
    handle: HDESK,
    /// `<window station>\<desktop>`, NUL-terminated, for `lpDesktop`.
    startup_name: Vec<u16>,
}

impl PrivateDesktop {
    pub(crate) fn create() -> Result<Self, String> {
        let name = format!(
            "HanafubukiAgentSandbox-{:x}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        );
        let wide = super::process::wide(&name);
        // SAFETY: `wide` is NUL-terminated; the handle is owned below.
        let handle = unsafe {
            CreateDesktopW(
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                DESKTOP_ALL_ACCESS,
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(last_error("CreateDesktopW"));
        }
        let desktop = Self {
            handle,
            startup_name: super::process::wide(&format!("{}\\{name}", window_station()?)),
        };
        desktop.grant_logon_session()?;
        Ok(desktop)
    }

    pub(crate) fn startup_name(&self) -> *mut u16 {
        self.startup_name.as_ptr().cast_mut()
    }

    /// A DACL of all access to the logon session alone, which the command's
    /// token keeps among its restricting SIDs.
    fn grant_logon_session(&self) -> Result<(), String> {
        let mut logon = super::token::current_logon_sid()?;
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: DESKTOP_ALL_ACCESS,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: 0,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: logon.as_mut_ptr().cast(),
            },
        };
        let mut updated: *mut ACL = std::ptr::null_mut();
        // SAFETY: the entry's SID outlives the call; `updated` is freed
        // below.
        let code = unsafe { SetEntriesInAclW(1, &entry, std::ptr::null(), &mut updated) };
        let result = if code != ERROR_SUCCESS {
            Err(format!("SetEntriesInAclW failed for the desktop: {code}"))
        } else {
            // SAFETY: the desktop handle has DESKTOP_WRITE_DAC.
            let code = unsafe {
                SetSecurityInfo(
                    self.handle as HANDLE,
                    SE_WINDOW_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    updated,
                    std::ptr::null(),
                )
            };
            if code == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(format!("SetSecurityInfo failed for the desktop: {code}"))
            }
        };
        // SAFETY: allocated by `SetEntriesInAclW`.
        unsafe { LocalFree(updated as HLOCAL) };
        result
    }
}

impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        // SAFETY: the handle is this value's.
        unsafe { CloseDesktop(self.handle) };
    }
}

/// The name of this process's window station (`WinSta0` in an interactive
/// session, another in a service's), where the desktop was created.
fn window_station() -> Result<String, String> {
    // SAFETY: the station handle is not owned and need not be closed.
    let station = unsafe { GetProcessWindowStation() };
    if station.is_null() {
        return Err(last_error("GetProcessWindowStation"));
    }
    let mut name = [0u16; 256];
    let mut needed = 0u32;
    // SAFETY: `name` holds the bytes its length says.
    let read = unsafe {
        GetUserObjectInformationW(
            station as HANDLE,
            UOI_NAME,
            name.as_mut_ptr().cast(),
            std::mem::size_of_val(&name) as u32,
            &mut needed,
        )
    };
    if read == 0 {
        return Err(last_error("GetUserObjectInformationW"));
    }
    let length = name
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(name.len());
    Ok(String::from_utf16_lossy(&name[..length]))
}
