// Derived from OpenAI Codex windows-sandbox-rs/src/wfp.rs at 57a808a392
// (Apache-2.0). Persistent per-account WFP blocks, without proxy exceptions.

use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    LocalFree, FWP_E_ALREADY_EXISTS, FWP_E_FILTER_NOT_FOUND, HANDLE,
};
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows_sys::Win32::Security::Authorization::{
    BuildExplicitAccessWithNameW, BuildSecurityDescriptorW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
};
use windows_sys::Win32::Security::PSECURITY_DESCRIPTOR;

use super::super::process::wide;

const PROVIDER: GUID = GUID::from_u128(0x58a4b3b6_7c47_4c83_90aa_f5e3da140a64);
const SUBLAYER: GUID = GUID::from_u128(0x31497012_f472_47b3_b032_829fe4f57ef2);

struct Engine(HANDLE);

impl Engine {
    fn open() -> Result<Self, String> {
        let mut handle = null_mut();
        // SAFETY: the local engine writes a new handle; no credentials supplied.
        check(
            unsafe { FwpmEngineOpen0(null(), 10, null(), null(), &mut handle) },
            "open WFP",
        )?;
        Ok(Self(handle))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: this value owns the engine handle.
        unsafe { FwpmEngineClose0(self.0) };
    }
}

struct UserCondition {
    descriptor: PSECURITY_DESCRIPTOR,
    blob: FWP_BYTE_BLOB,
}

impl UserCondition {
    fn new(account: &str) -> Result<Self, String> {
        let account = wide(account);
        let mut access: EXPLICIT_ACCESS_W = unsafe { std::mem::zeroed() };
        // SAFETY: the account and output structure outlive the call.
        unsafe {
            BuildExplicitAccessWithNameW(
                &mut access,
                account.as_ptr(),
                FWP_ACTRL_MATCH_FILTER,
                GRANT_ACCESS,
                0,
            );
        }
        let mut descriptor = null_mut();
        let mut size = 0;
        // SAFETY: the API allocates a self-relative descriptor, owned below.
        check(
            unsafe {
                BuildSecurityDescriptorW(
                    null(),
                    null(),
                    1,
                    &access,
                    0,
                    null(),
                    null_mut(),
                    &mut size,
                    &mut descriptor,
                )
            },
            "build WFP account condition",
        )?;
        Ok(Self {
            descriptor,
            blob: FWP_BYTE_BLOB {
                size,
                data: descriptor.cast(),
            },
        })
    }
}

impl Drop for UserCondition {
    fn drop(&mut self) {
        // SAFETY: BuildSecurityDescriptorW allocated this descriptor.
        unsafe { LocalFree(self.descriptor) };
    }
}

/// All IPv4/IPv6 connections, including loopback, are blocked for the offline
/// identity. Network-enabled commands use a different identity.
pub(super) fn install(account: &str, keys: &[GUID; 4]) -> Result<(), String> {
    let engine = Engine::open()?;
    let name = wide("Hanafubuki Agent sandbox");
    let mut provider: FWPM_PROVIDER0 = unsafe { std::mem::zeroed() };
    provider.providerKey = PROVIDER;
    provider.displayData.name = name.as_ptr().cast_mut();
    provider.flags = FWPM_PROVIDER_FLAG_PERSISTENT;
    // SAFETY: provider references live strings for this synchronous call.
    allow_existing(
        unsafe { FwpmProviderAdd0(engine.0, &provider, null_mut()) },
        "add WFP provider",
    )?;
    let mut sublayer: FWPM_SUBLAYER0 = unsafe { std::mem::zeroed() };
    sublayer.subLayerKey = SUBLAYER;
    sublayer.displayData.name = name.as_ptr().cast_mut();
    sublayer.flags = FWPM_SUBLAYER_FLAG_PERSISTENT;
    sublayer.providerKey = (&PROVIDER as *const GUID).cast_mut();
    sublayer.weight = 0x8000;
    // SAFETY: all pointers remain valid during the call.
    allow_existing(
        unsafe { FwpmSubLayerAdd0(engine.0, &sublayer, null_mut()) },
        "add WFP sublayer",
    )?;
    let user = UserCondition::new(account)?;
    let layers = [
        FWPM_LAYER_ALE_AUTH_CONNECT_V4,
        FWPM_LAYER_ALE_AUTH_CONNECT_V6,
        FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V4,
        FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V6,
    ];
    // Rebuild account conditions on repair: a recreated account has a new SID.
    check(
        unsafe { FwpmTransactionBegin0(engine.0, 0) },
        "begin WFP update",
    )?;
    for (key, layer) in keys.iter().zip(layers) {
        let removed = unsafe { FwpmFilterDeleteByKey0(engine.0, key) };
        if removed != FWP_E_FILTER_NOT_FOUND as u32 {
            check(removed, "replace sandbox network block")?;
        }
        let mut condition: FWPM_FILTER_CONDITION0 = unsafe { std::mem::zeroed() };
        condition.fieldKey = FWPM_CONDITION_ALE_USER_ID;
        condition.matchType = FWP_MATCH_EQUAL;
        condition.conditionValue.r#type = FWP_SECURITY_DESCRIPTOR_TYPE;
        condition.conditionValue.Anonymous.sd = (&user.blob as *const FWP_BYTE_BLOB).cast_mut();
        let mut filter: FWPM_FILTER0 = unsafe { std::mem::zeroed() };
        filter.filterKey = *key;
        filter.displayData.name = name.as_ptr().cast_mut();
        filter.flags = FWPM_FILTER_FLAG_PERSISTENT;
        filter.providerKey = (&PROVIDER as *const GUID).cast_mut();
        filter.layerKey = layer;
        filter.subLayerKey = SUBLAYER;
        filter.numFilterConditions = 1;
        filter.filterCondition = &mut condition;
        filter.action.r#type = FWP_ACTION_BLOCK;
        // SAFETY: the descriptor, condition and filter live through the call.
        check(
            unsafe { FwpmFilterAdd0(engine.0, &filter, null_mut(), null_mut()) },
            "block sandbox network",
        )?;
    }
    check(
        unsafe { FwpmTransactionCommit0(engine.0) },
        "commit WFP update",
    )
}

pub(super) fn verify(keys: &[GUID; 4]) -> Result<(), String> {
    let engine = Engine::open()?;
    for key in keys {
        let mut filter = null_mut();
        // SAFETY: the engine allocates the returned filter.
        check(
            unsafe { FwpmFilterGetByKey0(engine.0, key, &mut filter) },
            "read sandbox network block; run /setup-default-sandbox to repair",
        )?;
        // SAFETY: a successful query returned a filter; release with WFP's allocator.
        let blocks = unsafe { (*filter).action.r#type == FWP_ACTION_BLOCK };
        unsafe { FwpmFreeMemory0((&mut filter as *mut *mut FWPM_FILTER0).cast()) };
        if !blocks {
            return Err("the sandbox network block is missing; run /setup-default-sandbox".into());
        }
    }
    Ok(())
}

fn allow_existing(code: u32, operation: &str) -> Result<(), String> {
    if code == FWP_E_ALREADY_EXISTS as u32 {
        Ok(())
    } else {
        check(code, operation)
    }
}

fn check(code: u32, operation: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("{operation}: Windows error {code}"))
    }
}
