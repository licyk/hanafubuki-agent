// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/linux-sandbox/src/linux_run_main.rs` and
// `codex-rs/linux-sandbox/src/seccomp.rs` (the helper: confine the thread,
// then execute the command in place) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the filesystem is confined by Codex's
// legacy Landlock pipeline rather than bubblewrap, so there is no bundled
// `bwrap`, namespace, `/proc` mount or managed network proxy.

use std::os::unix::process::CommandExt;
use std::process::Command;

use crate::helper::HelperRequest;

pub(crate) mod bubblewrap;
mod filesystem;
mod network;

pub(crate) use network::seccomp_supported;

/// The kernel's Landlock ABI version: 0 or less when it has no Landlock or
/// has it disabled.
pub(crate) fn landlock_abi() -> i32 {
    const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1;
    // SAFETY: with a null attribute and size 0, the version flag only asks
    // the kernel for its ABI; nothing is read or written.
    let version = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    i32::try_from(version).unwrap_or(0)
}

/// Confines this process as `request` asks, then replaces it with the
/// command; returns only why that failed.
pub(crate) fn confine_and_exec(request: HelperRequest) -> String {
    if let Err(error) = confine(&request) {
        return error;
    }
    exec(request)
}

pub(crate) fn network_and_exec(request: HelperRequest) -> String {
    // SAFETY: no pointer arguments; the inner helper has no privileges to gain.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return format!(
            "cannot set no_new_privs: {}",
            std::io::Error::last_os_error()
        );
    }
    if let Err(error) = network::restrict_network(request.network_access) {
        return error;
    }
    exec(request)
}

fn exec(request: HelperRequest) -> String {
    let Some((program, args)) = request.command.split_first() else {
        return "no command to run".to_string();
    };
    let error = Command::new(program).args(args).exec();
    format!("cannot run {program:?}: {error}")
}

/// No new privileges (setuid binaries cannot escape), then the filesystem,
/// then the network. The helper is single-threaded here, so the calling
/// thread's restrictions are the process's, and its command inherits them.
fn confine(request: &HelperRequest) -> Result<(), String> {
    // SAFETY: `prctl(PR_SET_NO_NEW_PRIVS, 1)` takes no pointers.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!(
            "cannot set no_new_privs: {}",
            std::io::Error::last_os_error()
        ));
    }
    filesystem::restrict(&request.writable_roots, request.readable_roots.as_deref())?;
    network::restrict_network(request.network_access)
}
