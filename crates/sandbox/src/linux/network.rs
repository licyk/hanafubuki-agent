// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/linux-sandbox/src/seccomp.rs`
// (`install_network_seccomp_filter_on_current_thread`, its `Restricted`
// and `VmSocketRestricted` modes) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: no proxy-routed mode; the architecture is
// the running one, refused (not a panic) when seccompiler lacks it; errors
// are text for the helper's report.

use std::collections::BTreeMap;

use seccompiler::{
    apply_filter, BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition,
    SeccompFilter, SeccompRule, TargetArch,
};

/// Whether seccompiler can build a filter for this architecture.
pub(crate) fn seccomp_supported() -> bool {
    TargetArch::try_from(std::env::consts::ARCH).is_ok()
}

/// Installs this thread's network filter, whose denied calls fail with
/// `EPERM`. Without network access, every socket but a Unix one is
/// refused, and so are `connect`, `bind`, `listen`, `accept` and the other
/// calls of a connection (Unix sockets included), and `ptrace`. With it,
/// only VM sockets (`AF_VSOCK`), which reach the host past the sandbox, are
/// refused. `io_uring`, which can open sockets without `socket()`, is
/// refused either way.
pub(super) fn restrict_network(network_access: bool) -> Result<(), String> {
    let seccomp = |error: seccompiler::BackendError| format!("seccomp: {error}");
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    // An empty rule list matches every call.
    let mut deny = |call: libc::c_long| {
        rules.insert(call, Vec::new());
    };
    deny(libc::SYS_io_uring_setup);
    deny(libc::SYS_io_uring_enter);
    deny(libc::SYS_io_uring_register);
    if network_access {
        let vsock = SeccompRule::new(vec![SeccompCondition::new(
            0,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::Eq,
            libc::AF_VSOCK as u64,
        )
        .map_err(seccomp)?])
        .map_err(seccomp)?;
        rules.insert(libc::SYS_socket, vec![vsock.clone()]);
        rules.insert(libc::SYS_socketpair, vec![vsock]);
    } else {
        for call in [
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_connect,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_getpeername,
            libc::SYS_getsockname,
            libc::SYS_shutdown,
            libc::SYS_sendto,
            libc::SYS_sendmmsg,
            // `recvfrom` stays: tools such as `cargo clippy` talk to their
            // children over a socketpair.
            libc::SYS_recvmmsg,
            libc::SYS_getsockopt,
            libc::SYS_setsockopt,
        ] {
            deny(call);
        }
        let not_unix = SeccompRule::new(vec![SeccompCondition::new(
            0,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::Ne,
            libc::AF_UNIX as u64,
        )
        .map_err(seccomp)?])
        .map_err(seccomp)?;
        rules.insert(libc::SYS_socket, vec![not_unix.clone()]);
        rules.insert(libc::SYS_socketpair, vec![not_unix]);
    }
    let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(seccomp)?;
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        arch,
    )
    .map_err(seccomp)?;
    let program: BpfProgram = filter.try_into().map_err(seccomp)?;
    apply_filter(&program).map_err(|error| format!("seccomp: {error}"))
}
