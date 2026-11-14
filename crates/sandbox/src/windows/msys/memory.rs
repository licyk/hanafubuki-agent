use super::super::token::last_error;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Diagnostics::Debug::{
    FlushInstructionCache, ReadProcessMemory, WriteProcessMemory,
};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualProtectEx, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE, PAGE_READWRITE,
};

pub(super) fn read(process: HANDLE, address: usize, size: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0; size];
    let mut count = 0;
    // SAFETY: the OS checks the debuggee's range; the output holds `size` bytes.
    if unsafe {
        ReadProcessMemory(
            process,
            address as _,
            bytes.as_mut_ptr().cast(),
            size,
            &mut count,
        )
    } == 0
        || count != size
    {
        return Err(last_error("read MSYS debuggee memory"));
    }
    Ok(bytes)
}
pub(super) fn write(process: HANDLE, address: usize, bytes: &[u8]) -> Result<(), String> {
    let mut old = 0;
    // SAFETY: the OS checks the debuggee's range; callers supply an image slot or our allocation.
    if unsafe {
        VirtualProtectEx(
            process,
            address as _,
            bytes.len(),
            PAGE_EXECUTE_READWRITE,
            &mut old,
        )
    } == 0
    {
        return Err(last_error("protect MSYS debuggee memory"));
    }
    let mut count = 0;
    let written = unsafe {
        WriteProcessMemory(
            process,
            address as _,
            bytes.as_ptr().cast(),
            bytes.len(),
            &mut count,
        )
    };
    let mut ignored = 0;
    let restored =
        unsafe { VirtualProtectEx(process, address as _, bytes.len(), old, &mut ignored) };
    if written == 0 || count != bytes.len() || restored == 0 {
        return Err(last_error("update MSYS debuggee memory"));
    }
    if unsafe { FlushInstructionCache(process, address as _, bytes.len()) } == 0 {
        return Err(last_error("flush MSYS compatibility code"));
    }
    Ok(())
}
pub(super) fn allocate(process: HANDLE, size: usize) -> Result<usize, String> {
    // This allocation belongs to the job-owned debuggee and dies with it.
    let address = unsafe {
        VirtualAllocEx(
            process,
            std::ptr::null(),
            size,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if address.is_null() {
        return Err(last_error("allocate MSYS compatibility code"));
    }
    Ok(address as usize)
}
pub(super) fn executable(process: HANDLE, address: usize, size: usize) -> Result<(), String> {
    let mut old = 0;
    if unsafe { VirtualProtectEx(process, address as _, size, PAGE_EXECUTE_READ, &mut old) } == 0 {
        return Err(last_error("seal MSYS compatibility code"));
    }
    Ok(())
}
