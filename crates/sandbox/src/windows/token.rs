// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/windows-sandbox-rs/src/token.rs` (the restricted token: its
// restricting SIDs, default DACL and privileges) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: only the unelevated token, made from this
// process's own; a token with no capability SID (read-only) is allowed;
// handles are owned.

use std::ffi::c_void;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL, LUID,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, SetEntriesInAclW, EXPLICIT_ACCESS_W, GRANT_ACCESS, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, CopySid, CreateRestrictedToken, CreateWellKnownSid, GetLengthSid,
    GetTokenInformation, LookupPrivilegeValueW, SetTokenInformation, TokenDefaultDacl, TokenGroups,
    TokenLinkedToken, WinWorldSid, ACL, DISABLE_MAX_PRIVILEGE, LUA_TOKEN, SE_PRIVILEGE_ENABLED,
    SID_AND_ATTRIBUTES, TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY, TOKEN_DEFAULT_DACL, TOKEN_DUPLICATE,
    TOKEN_GROUPS, TOKEN_LINKED_TOKEN, TOKEN_PRIVILEGES, TOKEN_QUERY, WRITE_RESTRICTED,
};
use windows_sys::Win32::Storage::FileSystem::READ_CONTROL;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const GENERIC_ALL: u32 = 0x1000_0000;
const SE_GROUP_LOGON_ID: u32 = 0xC000_0000;
/// OWNER RIGHTS: an ACE for it replaces the owner's implicit rights.
const OWNER_RIGHTS: &str = "S-1-3-4";

/// A SID `ConvertStringSidToSidW` allocated, freed with `LocalFree`.
pub(crate) struct LocalSid(*mut c_void);

impl LocalSid {
    pub(crate) fn from_string(sid: &str) -> Result<Self, String> {
        let wide = super::process::wide(sid);
        let mut psid = std::ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated; on success `psid` is a SID this
        // value owns.
        if unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut psid) } == 0 {
            return Err(format!("invalid SID {sid}"));
        }
        Ok(Self(psid))
    }

    pub(crate) fn as_ptr(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for LocalSid {
    fn drop(&mut self) {
        // SAFETY: the SID was allocated by `ConvertStringSidToSidW`.
        unsafe { LocalFree(self.0 as HLOCAL) };
    }
}

/// A primary token like this process's, whose writes must also be allowed
/// to one of `capabilities`, the logon session or Everyone
/// (`WRITE_RESTRICTED`), with no privilege but traversal and, for an
/// administrator, the filtered (UAC) groups (`LUA_TOKEN`). Its default
/// DACL keeps the objects its processes create to its logon session.
pub(crate) fn restricted_token(capabilities: &[LocalSid]) -> Result<OwnedHandle, String> {
    let base = current_token()?;
    let mut logon = logon_sid(&base)?;
    let mut everyone = world_sid()?;
    let mut restricting: Vec<SID_AND_ATTRIBUTES> = capabilities
        .iter()
        .map(|sid| SID_AND_ATTRIBUTES {
            Sid: sid.as_ptr(),
            Attributes: 0,
        })
        .collect();
    for sid in [&mut logon, &mut everyone] {
        restricting.push(SID_AND_ATTRIBUTES {
            Sid: sid.as_mut_ptr().cast(),
            Attributes: 0,
        });
    }
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: every SID in `restricting` outlives the call; the new token is
    // owned below.
    let created = unsafe {
        CreateRestrictedToken(
            base.as_raw_handle() as HANDLE,
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            restricting.len() as u32,
            restricting.as_ptr(),
            &mut token,
        )
    };
    if created == 0 {
        return Err(last_error("CreateRestrictedToken"));
    }
    // SAFETY: `token` is a new handle no one else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token as _) };
    set_default_dacl(&token, &mut logon)?;
    enable_privilege(&token, "SeChangeNotifyPrivilege")?;
    Ok(token)
}

/// The logon session's SID of this process.
pub(crate) fn current_logon_sid() -> Result<Vec<u8>, String> {
    logon_sid(&current_token()?)
}

fn current_token() -> Result<OwnedHandle, String> {
    let access: TOKEN_ACCESS_MASK = TOKEN_DUPLICATE
        | TOKEN_QUERY
        | TOKEN_ASSIGN_PRIMARY
        | TOKEN_ADJUST_DEFAULT
        | TOKEN_ADJUST_SESSIONID
        | TOKEN_ADJUST_PRIVILEGES;
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle of this process; the token is owned below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut token) } == 0 {
        return Err(last_error("OpenProcessToken"));
    }
    // SAFETY: `token` is a new handle no one else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(token as _) })
}

/// The logon session's SID, from the token or, for an administrator whose
/// filtered token lacks it, from its linked token.
fn logon_sid(token: &OwnedHandle) -> Result<Vec<u8>, String> {
    if let Some(sid) = logon_sid_of(token.as_raw_handle() as HANDLE) {
        return Ok(sid);
    }
    let linked = token_information(token.as_raw_handle() as HANDLE, TokenLinkedToken)?;
    // SAFETY: the buffer holds a `TOKEN_LINKED_TOKEN`, whose handle is now
    // owned.
    let linked = unsafe {
        let linked = std::ptr::read_unaligned(linked.as_ptr().cast::<TOKEN_LINKED_TOKEN>());
        OwnedHandle::from_raw_handle(linked.LinkedToken as _)
    };
    logon_sid_of(linked.as_raw_handle() as HANDLE)
        .ok_or_else(|| "the token has no logon SID".to_string())
}

fn logon_sid_of(token: HANDLE) -> Option<Vec<u8>> {
    let buffer = token_information(token, TokenGroups).ok()?;
    // SAFETY: the buffer holds a `TOKEN_GROUPS` whose SIDs point into it.
    unsafe {
        let groups = buffer.as_ptr().cast::<TOKEN_GROUPS>();
        let count = (*groups).GroupCount as usize;
        let entries = std::ptr::addr_of!((*groups).Groups).cast::<SID_AND_ATTRIBUTES>();
        (0..count)
            .map(|index| *entries.add(index))
            .find(|group| group.Attributes & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID)
            .and_then(|group| copy_sid(group.Sid))
    }
}

/// # Safety
/// `sid` points to a valid SID.
unsafe fn copy_sid(sid: *mut c_void) -> Option<Vec<u8>> {
    let length = GetLengthSid(sid);
    let mut copy = vec![0u8; length as usize];
    (CopySid(length, copy.as_mut_ptr().cast(), sid) != 0).then_some(copy)
}

/// The `class` of `token`, in a buffer aligned for the structures it holds.
fn token_information(token: HANDLE, class: i32) -> Result<Vec<u64>, String> {
    let mut needed = 0u32;
    // SAFETY: a size query writes only `needed`.
    unsafe { GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(last_error("GetTokenInformation"));
    }
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: the buffer holds `needed` bytes.
    let read = unsafe {
        GetTokenInformation(
            token,
            class,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if read == 0 {
        return Err(last_error("GetTokenInformation"));
    }
    Ok(buffer)
}

fn world_sid() -> Result<Vec<u8>, String> {
    let mut size = 0u32;
    // SAFETY: a size query writes only `size`.
    unsafe {
        CreateWellKnownSid(
            WinWorldSid,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    let mut sid = vec![0u8; size as usize];
    // SAFETY: `sid` holds `size` bytes.
    let created = unsafe {
        CreateWellKnownSid(
            WinWorldSid,
            std::ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if created == 0 {
        return Err(last_error("CreateWellKnownSid"));
    }
    Ok(sid)
}

/// The DACL of what the token's processes create: all access to its logon
/// session, and to OWNER RIGHTS only reading it, so the shared owner cannot
/// rewrite it from another logon.
fn set_default_dacl(token: &OwnedHandle, logon: &mut [u8]) -> Result<(), String> {
    let owner_rights = LocalSid::from_string(OWNER_RIGHTS)?;
    let entries = [
        (logon.as_mut_ptr().cast::<c_void>(), GENERIC_ALL),
        (owner_rights.as_ptr(), READ_CONTROL),
    ]
    .map(|(sid, access)| EXPLICIT_ACCESS_W {
        grfAccessPermissions: access,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.cast(),
        },
    });
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: the entries' SIDs outlive the call; `dacl` is freed below.
    let code = unsafe {
        SetEntriesInAclW(
            entries.len() as u32,
            entries.as_ptr(),
            std::ptr::null(),
            &mut dacl,
        )
    };
    if code != ERROR_SUCCESS {
        return Err(format!("SetEntriesInAclW failed: {code}"));
    }
    let mut information = TOKEN_DEFAULT_DACL { DefaultDacl: dacl };
    // SAFETY: `information` points to the DACL just made.
    let set = unsafe {
        SetTokenInformation(
            token.as_raw_handle() as HANDLE,
            TokenDefaultDacl,
            std::ptr::addr_of_mut!(information).cast(),
            std::mem::size_of::<TOKEN_DEFAULT_DACL>() as u32,
        )
    };
    let result = if set == 0 {
        Err(last_error("SetTokenInformation(TokenDefaultDacl)"))
    } else {
        Ok(())
    };
    // SAFETY: `SetEntriesInAclW` allocated the DACL; the token keeps a copy.
    unsafe { LocalFree(dacl as HLOCAL) };
    result
}

fn enable_privilege(token: &OwnedHandle, name: &str) -> Result<(), String> {
    let wide = super::process::wide(name);
    let mut luid = LUID {
        LowPart: 0,
        HighPart: 0,
    };
    // SAFETY: `wide` is NUL-terminated.
    if unsafe { LookupPrivilegeValueW(std::ptr::null(), wide.as_ptr(), &mut luid) } == 0 {
        return Err(last_error("LookupPrivilegeValueW"));
    }
    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [windows_sys::Win32::Security::LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    // SAFETY: `privileges` is one well-formed entry.
    let adjusted = unsafe {
        AdjustTokenPrivileges(
            token.as_raw_handle() as HANDLE,
            0,
            &privileges,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    // It reports a privilege the token lacks only through the last error.
    let error = unsafe { GetLastError() };
    if adjusted == 0 || error != ERROR_SUCCESS {
        return Err(format!("AdjustTokenPrivileges failed: {error}"));
    }
    Ok(())
}

pub(crate) fn last_error(call: &str) -> String {
    format!("{call} failed: {}", std::io::Error::last_os_error())
}
