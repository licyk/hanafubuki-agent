// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/protocol/src/protocol.rs` (`SandboxPolicy` and its writable
// roots) and `codex-rs/config/src/config_toml.rs` (the policy a
// `sandbox_mode` and `[sandbox_workspace_write]` make) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the workspace is every workspace root
// rather than the working directory; read-only has no network switch; no
// external-sandbox policy, no `exclude_tmpdir_env_var` or
// `exclude_slash_tmp`; the read-only carve-outs under a writable root
// (`.git`) are the macOS and Windows backends', since Landlock cannot make
// them; Windows's sandbox is chosen by `[windows] sandbox`, as in Codex.

use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::agent_settings::{AgentSandboxMode, AgentWindowsSandbox};

/// What one command may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxPolicy {
    /// Read anything, write nothing (but `/dev/null` and `/dev/tty`), no
    /// network.
    ReadOnly,
    /// Read anything, write under `writable_roots`, network as allowed.
    WorkspaceWrite {
        writable_roots: Vec<PathBuf>,
        network_access: bool,
    },
    /// No sandbox.
    FullAccess,
}

impl SandboxPolicy {
    pub fn mode(&self) -> AgentSandboxMode {
        match self {
            Self::ReadOnly => AgentSandboxMode::ReadOnly,
            Self::WorkspaceWrite { .. } => AgentSandboxMode::WorkspaceWrite,
            Self::FullAccess => AgentSandboxMode::FullAccess,
        }
    }
}

/// The sandbox a turn's commands run in, frozen when the turn starts: the
/// session's mode (or the configured `sandboxMode`) with the configured
/// `[sandboxWorkspaceWrite]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxSettings {
    pub mode: AgentSandboxMode,
    /// Writable in workspace-write mode beside the workspace roots and the
    /// temporary directories.
    pub writable_roots: Vec<PathBuf>,
    /// Whether a workspace-write command may use the network.
    pub network_access: bool,
    /// The configured `[windows] sandbox`, which only Windows reads: none
    /// gives no sandbox there ([`crate::Sandbox::with_windows_sandbox`]).
    pub windows_sandbox: Option<AgentWindowsSandbox>,
}

impl SandboxSettings {
    /// No sandbox: a command no turn prepared asks for none.
    pub fn full_access() -> Self {
        Self {
            mode: AgentSandboxMode::FullAccess,
            writable_roots: Vec::new(),
            network_access: false,
            windows_sandbox: None,
        }
    }

    /// The policy of a command run in `workspace_roots`. In workspace-write
    /// mode they are writable, with the configured roots, the Agent's
    /// temporary directory (`TMPDIR`, on Windows `TEMP`) and on Unix `/tmp`,
    /// as in Codex.
    pub fn policy(&self, workspace_roots: &[PathBuf]) -> SandboxPolicy {
        match self.mode {
            AgentSandboxMode::ReadOnly => SandboxPolicy::ReadOnly,
            AgentSandboxMode::FullAccess => SandboxPolicy::FullAccess,
            AgentSandboxMode::WorkspaceWrite => {
                let mut writable_roots = Vec::new();
                let slash_tmp = cfg!(unix).then(|| Path::new("/tmp").to_path_buf());
                let temporary = std::env::temp_dir();
                for root in workspace_roots
                    .iter()
                    .chain(&self.writable_roots)
                    .chain(&slash_tmp)
                    .chain(std::iter::once(&temporary))
                {
                    if !writable_roots.contains(root) {
                        writable_roots.push(root.clone());
                    }
                }
                SandboxPolicy::WorkspaceWrite {
                    writable_roots,
                    network_access: self.network_access,
                }
            }
        }
    }
}

/// The mode an unset `sandboxMode` means: Codex's default preset,
/// workspace-write, where the platform's sandbox needs no setup (Linux,
/// macOS); elsewhere none, since a mode no backend can enforce would fail
/// every command. On Windows the sandbox is opt-in (`[windows] sandbox`),
/// as in Codex, and the restricted token it has does not block the
/// network.
pub fn platform_default_mode() -> AgentSandboxMode {
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        AgentSandboxMode::WorkspaceWrite
    } else {
        AgentSandboxMode::FullAccess
    }
}

#[cfg(test)]
mod tests;
