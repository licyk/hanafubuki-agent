// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/windows-sandbox-rs/src/acl.rs` (`ensure_allow_write_aces`,
// `add_deny_write_ace`, `allow_null_device` and the DACL checks they make)
// at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: only the inheriting write grant and deny a
// command's roots need; one SID at a time; the null device is opened as
// `\\.\NUL`.

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::{
    GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, ACCESS_MODE, DENY_ACCESS,
    EXPLICIT_ACCESS_W, SET_ACCESS, SE_FILE_OBJECT, SE_KERNEL_OBJECT, SE_OBJECT_TYPE,
    TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    AclSizeInformation, EqualSid, GetAce, GetAclInformation, MapGenericMask, ACCESS_ALLOWED_ACE,
    ACE_HEADER, ACL, ACL_SIZE_INFORMATION, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION,
    GENERIC_MAPPING, INHERITED_ACE, INHERIT_ONLY_ACE, OBJECT_INHERIT_ACE,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA,
    READ_CONTROL, WRITE_DAC,
};

use super::token::LocalSid;

const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const GENERIC_WRITE: u32 = 0x4000_0000;
const INHERIT: u32 = CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE;

/// What a writable root grants its capability SID, down the tree. `DELETE`
/// on each descendant rather than `FILE_DELETE_CHILD` on its parent, which
/// would let a command delete a read-only child (`.git`) past its deny.
const WRITE_ALLOW_MASK: u32 =
    FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE;

/// What a read-only path denies the capability SIDs of the roots holding
/// it, down the tree.
const DENY_WRITE_MASK: u32 = FILE_GENERIC_WRITE
    | FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | GENERIC_WRITE
    | DELETE
    | FILE_DELETE_CHILD;

const FILE_MAPPING: GENERIC_MAPPING = GENERIC_MAPPING {
    GenericRead: FILE_GENERIC_READ,
    GenericWrite: FILE_GENERIC_WRITE,
    GenericExecute: FILE_GENERIC_EXECUTE,
    GenericAll: FILE_ALL_ACCESS,
};

/// Grants `sid` [`WRITE_ALLOW_MASK`] on `path` and everything under it,
/// unless it has it already (and no explicit `FILE_DELETE_CHILD` an older
/// grant gave).
pub(crate) fn ensure_allow_write_ace(path: &Path, sid: &LocalSid) -> Result<(), String> {
    let dacl = Dacl::of(&open(path, READ_CONTROL)?, SE_FILE_OBJECT)?;
    if dacl.allows(sid, WRITE_ALLOW_MASK) && !dacl.explicitly_allows_any(sid, FILE_DELETE_CHILD) {
        return Ok(());
    }
    let file = open(path, READ_CONTROL | WRITE_DAC)?;
    dacl.set_with(&file, SE_FILE_OBJECT, sid, WRITE_ALLOW_MASK, SET_ACCESS)
}

/// Denies `sid` writing `path` and everything under it, unless it is denied
/// already (then a path it cannot change the DACL of is fine).
pub(crate) fn add_deny_write_ace(path: &Path, sid: &LocalSid) -> Result<(), String> {
    let writable = open(path, READ_CONTROL | WRITE_DAC);
    let file = match &writable {
        Ok(file) => file,
        Err(_) => &open(path, READ_CONTROL)?,
    };
    let dacl = Dacl::of(file, SE_FILE_OBJECT)?;
    if dacl.denies(sid, DENY_WRITE_MASK) {
        return Ok(());
    }
    let file = writable?;
    dacl.set_with(&file, SE_FILE_OBJECT, sid, DENY_WRITE_MASK, DENY_ACCESS)
}

/// Lets `sid` read and write the null device, for redirections to `NUL`;
/// a machine that refuses it keeps its own DACL there, as in Codex.
pub(crate) fn allow_null_device(sid: &LocalSid) {
    let Ok(device) = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(r"\\.\NUL")
    else {
        return;
    };
    let Ok(dacl) = Dacl::of(&device, SE_KERNEL_OBJECT) else {
        return;
    };
    let mask = FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE;
    if dacl.allows(sid, mask) {
        return;
    }
    let _ = dacl.set_entry(&device, SE_KERNEL_OBJECT, sid, mask, SET_ACCESS, 0);
}

fn open(path: &Path, access: u32) -> Result<File, String> {
    OpenOptions::new()
        .access_mode(access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|error| format!("cannot open its ACL: {error}"))
}

/// An object's DACL, with the security descriptor that holds it.
struct Dacl {
    descriptor: *mut c_void,
    acl: *mut ACL,
}

impl Dacl {
    fn of(object: &File, kind: SE_OBJECT_TYPE) -> Result<Self, String> {
        let mut descriptor = std::ptr::null_mut();
        let mut acl = std::ptr::null_mut();
        // SAFETY: the handle is open with READ_CONTROL; the descriptor is
        // freed when this value drops.
        let code = unsafe {
            GetSecurityInfo(
                object.as_raw_handle() as HANDLE,
                kind,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut acl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if code != ERROR_SUCCESS {
            return Err(format!("GetSecurityInfo failed: {code}"));
        }
        Ok(Self { descriptor, acl })
    }

    /// Whether an allow ACE for `sid` that applies to the object grants
    /// all of `mask`.
    fn allows(&self, sid: &LocalSid, mask: u32) -> bool {
        self.aces(ACCESS_ALLOWED_ACE_TYPE)
            .any(|(_, ace_mask, ace_sid)| equal(ace_sid, sid) && ace_mask & mask == mask)
    }

    /// Whether an explicit (not inherited) allow ACE for `sid` grants any
    /// of `mask`.
    fn explicitly_allows_any(&self, sid: &LocalSid, mask: u32) -> bool {
        self.aces(ACCESS_ALLOWED_ACE_TYPE)
            .any(|(flags, ace_mask, ace_sid)| {
                flags & INHERITED_ACE as u8 == 0 && equal(ace_sid, sid) && ace_mask & mask != 0
            })
    }

    /// Whether a deny ACE for `sid` denies any of `mask`.
    fn denies(&self, sid: &LocalSid, mask: u32) -> bool {
        self.aces(ACCESS_DENIED_ACE_TYPE)
            .any(|(_, ace_mask, ace_sid)| equal(ace_sid, sid) && ace_mask & mask != 0)
    }

    /// `(flags, generic-mapped mask, SID)` of the ACEs of `kind` that apply
    /// to the object itself (not inherit-only).
    fn aces(&self, kind: u8) -> impl Iterator<Item = (u8, u32, *mut c_void)> + '_ {
        let count = self.count();
        (0..count).filter_map(move |index| {
            let mut ace = std::ptr::null_mut();
            // SAFETY: `index` is below the ACE count; an allowed and a
            // denied ACE share their layout, the SID following the mask.
            unsafe {
                if GetAce(self.acl, index, &mut ace) == 0 {
                    return None;
                }
                let header = &*(ace as *const ACE_HEADER);
                if header.AceType != kind || header.AceFlags & INHERIT_ONLY_ACE as u8 != 0 {
                    return None;
                }
                let ace = ace as *mut ACCESS_ALLOWED_ACE;
                let mut mask = (*ace).Mask;
                let raw = mask;
                MapGenericMask(&mut mask, &FILE_MAPPING);
                let sid = std::ptr::addr_of_mut!((*ace).SidStart).cast::<c_void>();
                Some((header.AceFlags, mask | raw, sid))
            }
        })
    }

    fn count(&self) -> u32 {
        if self.acl.is_null() {
            return 0;
        }
        let mut information = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        // SAFETY: `acl` is the descriptor's DACL.
        let read = unsafe {
            GetAclInformation(
                self.acl,
                std::ptr::addr_of_mut!(information).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        };
        if read == 0 {
            0
        } else {
            information.AceCount
        }
    }

    /// Writes this DACL with an inheriting entry for `sid` added.
    fn set_with(
        &self,
        object: &File,
        kind: SE_OBJECT_TYPE,
        sid: &LocalSid,
        mask: u32,
        mode: ACCESS_MODE,
    ) -> Result<(), String> {
        self.set_entry(object, kind, sid, mask, mode, INHERIT)
    }

    fn set_entry(
        &self,
        object: &File,
        kind: SE_OBJECT_TYPE,
        sid: &LocalSid,
        mask: u32,
        mode: ACCESS_MODE,
        inheritance: u32,
    ) -> Result<(), String> {
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: mask,
            grfAccessMode: mode,
            grfInheritance: inheritance,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: sid.as_ptr().cast(),
            },
        };
        let mut updated: *mut ACL = std::ptr::null_mut();
        // SAFETY: the entry's SID outlives the call; `updated` is freed
        // below. A new deny goes before the allows, as Windows orders them.
        let code = unsafe { SetEntriesInAclW(1, &entry, self.acl, &mut updated) };
        if code != ERROR_SUCCESS {
            return Err(format!("SetEntriesInAclW failed: {code}"));
        }
        // SAFETY: the handle is open with WRITE_DAC.
        let code = unsafe {
            SetSecurityInfo(
                object.as_raw_handle() as HANDLE,
                kind,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                updated,
                std::ptr::null(),
            )
        };
        // SAFETY: `SetEntriesInAclW` allocated it.
        unsafe { LocalFree(updated as HLOCAL) };
        if code != ERROR_SUCCESS {
            return Err(format!("SetSecurityInfo failed: {code}"));
        }
        Ok(())
    }
}

impl Drop for Dacl {
    fn drop(&mut self) {
        // SAFETY: `GetSecurityInfo` allocated the descriptor.
        unsafe { LocalFree(self.descriptor as HLOCAL) };
    }
}

fn equal(ace_sid: *mut c_void, sid: &LocalSid) -> bool {
    // SAFETY: both are valid SIDs.
    unsafe { EqualSid(ace_sid, sid.as_ptr()) != 0 }
}
