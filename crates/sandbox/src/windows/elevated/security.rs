// Adapted from Codex windows-sandbox-rs/identity.rs and setup.rs (57a808a392,
// Apache-2.0): separate account SIDs and owner-only credential storage.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, LookupAccountNameW, SetFileSecurityW, TokenUser,
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::super::process::wide;
use super::super::token::{last_error, LocalSid};

pub(super) fn current_user_sid() -> Result<String, String> {
    let mut token = null_mut();
    // SAFETY: opens this process's token, owned below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("OpenProcessToken"));
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token as _) };
    let mut needed = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle() as HANDLE,
            TokenUser,
            null_mut(),
            0,
            &mut needed,
        )
    };
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle() as HANDLE,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(last_error("GetTokenInformation"));
    }
    // SAFETY: the buffer contains TOKEN_USER and its SID.
    sid_string(unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid })
}

pub(super) fn account_sid(account: &str) -> Result<LocalSid, String> {
    let name = wide(account);
    let (mut sid_size, mut domain_size, mut kind) = (0, 0, 0);
    // SAFETY: a size query; the API writes only the sizes and kind.
    unsafe {
        LookupAccountNameW(
            null(),
            name.as_ptr(),
            null_mut(),
            &mut sid_size,
            null_mut(),
            &mut domain_size,
            &mut kind,
        )
    };
    if sid_size == 0 {
        return Err(last_error("find sandbox account"));
    }
    let mut sid = vec![0u64; (sid_size as usize).div_ceil(8)];
    let mut domain = vec![0u16; domain_size as usize];
    if unsafe {
        LookupAccountNameW(
            null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut kind,
        )
    } == 0
    {
        return Err(last_error("find sandbox account"));
    }
    LocalSid::from_string(&sid_string(sid.as_mut_ptr().cast())?)
}

pub(super) fn sid_string(sid: *mut std::ffi::c_void) -> Result<String, String> {
    let mut text = null_mut();
    // SAFETY: callers supply a SID returned by the Windows token/account APIs.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(last_error("ConvertSidToStringSidW"));
    }
    let mut length = 0;
    // SAFETY: the API returned a NUL-terminated UTF-16 allocation.
    unsafe {
        while *text.add(length) != 0 {
            length += 1;
        }
    }
    let result = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(text, length)) };
    unsafe { LocalFree(text.cast()) };
    Ok(result)
}

pub(super) fn protect_secrets(path: &Path, owner: &str) -> Result<(), String> {
    // Validate the SID before interpolating it into an SDDL descriptor.
    let _ = LocalSid::from_string(owner)?;
    let sddl = wide(&format!(
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{owner})"
    ));
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
        return Err(last_error("build credential DACL"));
    }
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the descriptor is valid and the path is NUL-terminated.
    let ok = unsafe {
        SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    unsafe { LocalFree(descriptor) };
    if ok == 0 {
        Err(last_error("protect sandbox credentials"))
    } else {
        Ok(())
    }
}

pub(super) fn enable_standard_account(account: &str) -> Result<(), String> {
    use windows_sys::Win32::NetworkManagement::NetManagement::{
        NetLocalGroupAddMembers, LOCALGROUP_MEMBERS_INFO_0,
    };
    use windows_sys::Win32::Security::LookupAccountSidW;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_DWORD,
    };
    let users = LocalSid::from_string("S-1-5-32-545")?;
    let (mut name_len, mut domain_len, mut kind) = (0, 0, 0);
    unsafe {
        LookupAccountSidW(
            null(),
            users.as_ptr(),
            null_mut(),
            &mut name_len,
            null_mut(),
            &mut domain_len,
            &mut kind,
        )
    };
    let mut name = vec![0u16; name_len as usize];
    let mut domain = vec![0u16; domain_len as usize];
    if unsafe {
        LookupAccountSidW(
            null(),
            users.as_ptr(),
            name.as_mut_ptr(),
            &mut name_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut kind,
        )
    } == 0
    {
        return Err(last_error("find the local Users group"));
    }
    let sid = account_sid(account)?;
    let member = LOCALGROUP_MEMBERS_INFO_0 {
        lgrmi0_sid: sid.as_ptr(),
    };
    let result = unsafe {
        NetLocalGroupAddMembers(
            null(),
            name.as_ptr(),
            0,
            (&member as *const LOCALGROUP_MEMBERS_INFO_0).cast(),
            1,
        )
    };
    if result != 0 && result != 1378 {
        return Err(format!(
            "add sandbox account to Users: Windows error {result}"
        ));
    }
    let key =
        wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList");
    let mut handle = null_mut();
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut handle,
            null_mut(),
        )
    };
    if result != 0 {
        return Err(format!(
            "hide sandbox sign-in account: Windows error {result}"
        ));
    }
    let account = wide(account);
    let hidden = 0u32;
    let result = unsafe {
        RegSetValueExW(
            handle,
            account.as_ptr(),
            0,
            REG_DWORD,
            (&hidden as *const u32).cast(),
            4,
        )
    };
    unsafe { RegCloseKey(handle) };
    if result != 0 {
        return Err(format!(
            "hide sandbox sign-in account: Windows error {result}"
        ));
    }
    Ok(())
}
