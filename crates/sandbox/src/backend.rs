// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/sandboxing/src/manager.rs` (the platform's sandbox and the
// command's transformation) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: Landlock rather than bubblewrap on Linux,
// probed once; Seatbelt on macOS when `/usr/bin/sandbox-exec` exists; on
// Windows only the unelevated (restricted token) sandbox, through the
// helper, and only when `[windows] sandbox` chooses it; a policy no backend
// can enforce is an error, never a command run unconfined; no network
// proxy.

use std::fmt;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::agent_settings::{AgentSandboxMode, AgentWindowsSandbox};

use crate::helper::HelperRequest;
use crate::SandboxPolicy;

/// The platform's sandbox: what enforces a [`SandboxPolicy`] on this
/// machine, or why nothing can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    backend: Backend,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Backend {
    /// This program as the helper, which confines itself with `confinement`
    /// (Linux: Landlock and seccomp) before it runs the command, or (Windows)
    /// runs it with a restricted token.
    #[cfg_attr(
        not(any(target_os = "linux", windows)),
        allow(dead_code, reason = "only Linux and Windows have a helper backend")
    )]
    Helper {
        helper: PathBuf,
        confinement: String,
    },
    /// `/usr/bin/sandbox-exec` with a Seatbelt policy (macOS).
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only macOS has Seatbelt")
    )]
    Seatbelt,
    Unavailable {
        reason: String,
    },
}

/// What [`Sandbox::status`] reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxStatus {
    Available { backend: String },
    Unavailable { reason: String },
}

/// A command that asked for a sandbox this machine cannot give it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxError {
    Unavailable {
        mode: AgentSandboxMode,
        reason: String,
    },
    /// A path the helper's command line cannot carry.
    NotUtf8,
}

impl fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { mode, reason } => write!(
                formatter,
                "the command was not run: it needs the {} sandbox, which is unavailable ({reason}); \
                 set sandboxMode to \"full-access\" to run commands without a sandbox",
                mode.as_str()
            ),
            Self::NotUtf8 => formatter
                .write_str("the command was not run: a sandboxed command and its writable roots must be UTF-8"),
        }
    }
}

impl std::error::Error for SandboxError {}

impl Sandbox {
    /// The sandbox this machine has. On Linux: Landlock (Linux 5.13 or
    /// later, enabled) and seccomp on x86_64 or aarch64, run by this
    /// program as the helper once it called
    /// [`helper_dispatch`](crate::helper_dispatch). On macOS: Seatbelt,
    /// through `/usr/bin/sandbox-exec`. On Windows: a restricted token, run
    /// by the helper likewise, which [`Self::with_windows_sandbox`] then
    /// turns on.
    pub fn detect() -> Self {
        Self {
            backend: detect_backend(),
        }
    }

    /// No sandbox, for `reason`: every policy but full access fails.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            backend: Backend::Unavailable {
                reason: reason.into(),
            },
        }
    }

    /// This sandbox as `[windows] sandbox` (`setting`) has it on Windows:
    /// unset, none (the sandbox is opt-in there, as in Codex); `elevated`,
    /// none yet; `unelevated`, the restricted token. Elsewhere it changes
    /// nothing.
    pub fn with_windows_sandbox(&self, setting: Option<AgentWindowsSandbox>) -> Self {
        if !cfg!(windows) || matches!(self.backend, Backend::Unavailable { .. }) {
            return self.clone();
        }
        match setting {
            Some(AgentWindowsSandbox::Unelevated) => self.clone(),
            Some(AgentWindowsSandbox::Elevated) => Self::unavailable(
                "the elevated Windows sandbox is not available yet; \
                 set windows.sandbox to \"unelevated\" for the restricted-token sandbox",
            ),
            None => Self::unavailable(
                "the Windows sandbox is off; set windows.sandbox to \"unelevated\" to turn it on",
            ),
        }
    }

    pub fn status(&self) -> SandboxStatus {
        match &self.backend {
            Backend::Helper { confinement, .. } => SandboxStatus::Available {
                backend: confinement.clone(),
            },
            Backend::Seatbelt => SandboxStatus::Available {
                backend: format!("Seatbelt ({})", crate::macos::SANDBOX_EXEC),
            },
            Backend::Unavailable { reason } => SandboxStatus::Unavailable {
                reason: reason.clone(),
            },
        }
    }

    /// `command` (argv) confined by `policy`: unchanged under full access,
    /// else the command line that runs it confined (the helper's, or
    /// `sandbox-exec`'s). A policy this machine cannot enforce is an error,
    /// so the command is never run unconfined.
    pub fn wrap(
        &self,
        policy: &SandboxPolicy,
        command: Vec<String>,
    ) -> Result<Vec<String>, SandboxError> {
        let (writable_roots, network_access) = match policy {
            SandboxPolicy::FullAccess => return Ok(command),
            SandboxPolicy::ReadOnly => (Vec::new(), false),
            SandboxPolicy::WorkspaceWrite {
                writable_roots,
                network_access,
            } => (writable_roots.clone(), *network_access),
        };
        match &self.backend {
            Backend::Helper { helper, .. } => helper_command(
                helper,
                HelperRequest {
                    writable_roots,
                    network_access,
                    command: command.into_iter().map(Into::into).collect(),
                },
            ),
            Backend::Seatbelt => Ok(crate::macos::command_line(
                &writable_roots,
                network_access,
                command,
            )),
            Backend::Unavailable { reason } => Err(SandboxError::Unavailable {
                mode: policy.mode(),
                reason: reason.clone(),
            }),
        }
    }
}

fn helper_command(helper: &Path, request: HelperRequest) -> Result<Vec<String>, SandboxError> {
    let mut command = vec![
        helper.to_str().ok_or(SandboxError::NotUtf8)?.to_string(),
        crate::HELPER_ARG.to_string(),
    ];
    command.extend(request.to_args().ok_or(SandboxError::NotUtf8)?);
    Ok(command)
}

#[cfg(target_os = "linux")]
fn detect_backend() -> Backend {
    let Some(helper) = crate::helper::registered_helper() else {
        return Backend::Unavailable {
            reason: "this program does not run the sandbox helper".to_string(),
        };
    };
    linux_backend(helper.clone(), crate::linux::landlock_abi())
}

#[cfg(target_os = "macos")]
fn detect_backend() -> Backend {
    if Path::new(crate::macos::SANDBOX_EXEC).is_file() {
        Backend::Seatbelt
    } else {
        Backend::Unavailable {
            reason: format!("{} is missing", crate::macos::SANDBOX_EXEC),
        }
    }
}

#[cfg(windows)]
fn detect_backend() -> Backend {
    match crate::helper::registered_helper() {
        Some(helper) => Backend::Helper {
            helper: helper.clone(),
            confinement: "a restricted token (unelevated: writes are confined, \
                          the network is not blocked)"
                .to_string(),
        },
        None => Backend::Unavailable {
            reason: "this program does not run the sandbox helper".to_string(),
        },
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn detect_backend() -> Backend {
    Backend::Unavailable {
        reason: "the sandbox is not available on this platform yet".to_string(),
    }
}

/// The Linux backend over a kernel whose Landlock ABI is `abi` (0 or less
/// when the kernel lacks Landlock or has it disabled).
#[cfg(target_os = "linux")]
fn linux_backend(helper: PathBuf, abi: i32) -> Backend {
    if abi < 1 {
        return Backend::Unavailable {
            reason: "this kernel has no Landlock (Linux 5.13 or later, with Landlock enabled)"
                .to_string(),
        };
    }
    if !crate::linux::seccomp_supported() {
        return Backend::Unavailable {
            reason: format!(
                "the seccomp filter does not support {}",
                std::env::consts::ARCH
            ),
        };
    }
    Backend::Helper {
        helper,
        confinement: format!("Landlock (ABI {abi}) and seccomp"),
    }
}

#[cfg(test)]
mod tests;
