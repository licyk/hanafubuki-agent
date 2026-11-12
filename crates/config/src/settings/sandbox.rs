//! `sandboxMode`, `[sandboxWorkspaceWrite]` and `[windows]`: what the
//! commands the model runs can reach.

use std::path::PathBuf;

use hanafubuki_agent_foundation::settings::{SettingChoice, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};

use super::AgentSandboxMode;
use crate::fields::FieldRead;
use crate::sections::{described, SettingsGroup, SettingsRead, SettingsSection};

const SANDBOX_MODE: &str = "sandboxMode";

/// Codex's `[sandbox_workspace_write]`: what workspace-write allows beside
/// the workspace roots and the temporary directories.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSandboxWorkspaceWrite {
    /// More directories commands may write in, each absolute.
    pub writable_roots: Vec<PathBuf>,
    /// Whether commands may use the network.
    pub network_access: bool,
}

/// Codex's `[windows]`: the Windows sandbox `sandboxMode` runs in. Unset,
/// a mode other than full access cannot be given on Windows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentWindowsSettings {
    pub sandbox: Option<super::AgentWindowsSandbox>,
}

pub(crate) fn section() -> SettingsSection {
    let choice = |values: &[(&str, &str)]| SettingKind::Choice {
        values: values
            .iter()
            .map(|(value, label)| SettingChoice::new(*value, *label))
            .collect(),
    };
    SettingsSection::new(
        "sandbox",
        "Sandbox",
        SettingsGroup::General,
        20,
        vec![
            described(
                SettingSpec::new(
                    "sandboxMode",
                    "Sandbox mode",
                    choice(&[
                        ("read-only", "Read only"),
                        ("workspace-write", "Workspace write"),
                        ("full-access", "Full access"),
                    ]),
                ),
                "What the commands the model runs can reach; unset, the platform's default",
            ),
            SettingSpec {
                advanced: true,
                ..described(
                    SettingSpec::new(
                        "sandboxWorkspaceWrite.writableRoots",
                        "Writable roots",
                        SettingKind::TextList {
                            maximum_items: None,
                            maximum_item_chars: None,
                        },
                    ),
                    "More directories workspace-write lets commands write in, each absolute",
                )
            },
            described(
                SettingSpec::new(
                    "sandboxWorkspaceWrite.networkAccess",
                    "Network access",
                    SettingKind::Boolean,
                ),
                "Whether workspace-write lets commands use the network",
            ),
            described(
                SettingSpec::new(
                    "windows.sandbox",
                    "Windows sandbox",
                    choice(&[("unelevated", "Unelevated"), ("elevated", "Elevated")]),
                ),
                "The Windows sandbox; other platforms ignore it",
            ),
        ],
    )
}

/// A mode that cannot be read is read-only, the most restrictive: the user
/// asked for some sandbox, and an unreadable one never grants more.
pub(crate) fn decode_sandbox_mode(
    read: &mut SettingsRead<'_>,
    default: Option<AgentSandboxMode>,
) -> Option<AgentSandboxMode> {
    match read
        .fields()
        .read_quiet::<AgentSandboxMode>(SANDBOX_MODE, Ok)
    {
        FieldRead::Valid(mode) => {
            read.mark(SANDBOX_MODE);
            Some(mode)
        }
        FieldRead::Missing => default,
        FieldRead::Invalid => {
            read.fields().invalid(
                SANDBOX_MODE,
                Some("the sandbox mode is not supported; commands run read-only until one is chosen again"),
            );
            read.mark(SANDBOX_MODE);
            Some(AgentSandboxMode::ReadOnly)
        }
    }
}

/// A list holding a relative root is dropped whole, which leaves fewer
/// places writable, never more.
pub(crate) fn validate_sandbox_workspace_write(
    settings: &AgentSandboxWorkspaceWrite,
) -> Result<(), String> {
    if settings
        .writable_roots
        .iter()
        .any(|root| !root.is_absolute())
    {
        return Err("sandboxWorkspaceWrite.writableRoots must hold absolute paths".to_string());
    }
    Ok(())
}
