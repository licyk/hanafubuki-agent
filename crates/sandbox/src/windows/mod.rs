// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/windows-sandbox-rs/src/lib.rs`
// (`run_windows_sandbox_capture_with_filesystem_overrides`) and
// `src/spawn_prep.rs` (the legacy, unelevated session: its token, its ACL
// rules, its offline environment) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the helper runs one command with its own
// standard handles and returns its exit status, rather than capturing its
// output; an ACE a read-only path needs that cannot be added refuses the
// command rather than being ignored; no deny-read paths, logs or
// `CODEX_HOME`.

mod plan;

#[cfg(windows)]
mod acl;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod process;
#[cfg(windows)]
mod token;

/// Runs `request`'s command confined and returns its exit status, or why
/// it could not start; a command that cannot be confined is not started.
///
/// The token restricts writes to the capability SIDs of the writable roots
/// (each granted on its root), the logon session and Everyone; the command
/// reads what its user can. The network is not blocked: without network
/// access the command only gets an offline environment.
#[cfg(windows)]
pub(crate) fn run(request: crate::helper::HelperRequest) -> Result<u32, String> {
    let command = request
        .command
        .iter()
        .map(|arg| arg.to_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()
        .ok_or("the command must be UTF-8")?;
    let command_line = plan::command_line(&command)?;
    let plan = plan::acl_plan(&crate::roots::resolve_roots(&request.writable_roots));
    let capabilities = plan
        .grants
        .iter()
        .map(|grant| token::LocalSid::from_string(&grant.sid))
        .collect::<Result<Vec<_>, _>>()?;
    for deny in &plan.denies {
        for &grant in &deny.grants {
            acl::add_deny_write_ace(deny.path.system(), &capabilities[grant]).map_err(|error| {
                format!("cannot keep {} read-only: {error}", deny.path.native_text())
            })?;
        }
    }
    for (grant, sid) in plan.grants.iter().zip(&capabilities) {
        // A root left without its ACE is only unwritable, as in Codex.
        let _ = acl::ensure_allow_write_ace(grant.root.system(), sid);
    }
    let token = token::restricted_token(&capabilities)?;
    for sid in &capabilities {
        acl::allow_null_device(sid);
    }
    if !request.network_access {
        for (key, value) in plan::offline_environment(|key| std::env::var_os(key).is_some()) {
            std::env::set_var(key, value);
        }
    }
    let desktop = desktop::PrivateDesktop::create()?;
    process::run(&token, &command_line, &desktop)
}
