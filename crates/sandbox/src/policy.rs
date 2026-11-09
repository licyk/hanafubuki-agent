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
// (`.git`) are the macOS backend's, since Landlock cannot make them.

use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::agent_settings::AgentSandboxMode;

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
}

impl SandboxSettings {
    /// No sandbox: a command no turn prepared asks for none.
    pub fn full_access() -> Self {
        Self {
            mode: AgentSandboxMode::FullAccess,
            writable_roots: Vec::new(),
            network_access: false,
        }
    }

    /// The policy of a command run in `workspace_roots`. In workspace-write
    /// mode they are writable, with the configured roots, `/tmp` and the
    /// Agent's temporary directory (`TMPDIR`), as in Codex.
    pub fn policy(&self, workspace_roots: &[PathBuf]) -> SandboxPolicy {
        match self.mode {
            AgentSandboxMode::ReadOnly => SandboxPolicy::ReadOnly,
            AgentSandboxMode::FullAccess => SandboxPolicy::FullAccess,
            AgentSandboxMode::WorkspaceWrite => {
                let mut writable_roots = Vec::new();
                let temporary = [Path::new("/tmp").to_path_buf(), std::env::temp_dir()];
                for root in workspace_roots
                    .iter()
                    .chain(&self.writable_roots)
                    .chain(temporary.iter().filter(|_| cfg!(unix)))
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
/// workspace-write, where the platform has a sandbox (Linux, macOS);
/// elsewhere none, since a mode no backend can enforce would fail every
/// command. Windows has no backend yet.
pub fn platform_default_mode() -> AgentSandboxMode {
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        AgentSandboxMode::WorkspaceWrite
    } else {
        AgentSandboxMode::FullAccess
    }
}

#[cfg(test)]
mod tests;
