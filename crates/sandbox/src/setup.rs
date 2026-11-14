//! Explicit setup entry point for standalone and embedded frontends.

/// Installs the elevated Windows sandbox after Windows asks for administrator
/// consent. Blocks until setup ends; call it off the frontend's event loop.
/// Configuration is changed by the caller only after this succeeds.
#[cfg(windows)]
pub fn setup_windows_sandbox() -> Result<(), String> {
    crate::windows::elevated::setup()
}

#[cfg(not(windows))]
pub fn setup_windows_sandbox() -> Result<(), String> {
    Err("Windows sandbox setup is only available on Windows".into())
}
