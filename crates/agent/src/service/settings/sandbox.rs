//! The sandbox's status and explicit provisioning operation.
use super::record::record_of;
use super::{SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection};
use crate::service::AgentApplicationService;
use hanafubuki_agent_config::{SettingChange, SettingWriteOp, SettingsGroup, SettingsWrite};
use hanafubuki_agent_sandbox::SandboxStatus;

pub(super) const ID: &str = "sandboxMaintenance";

pub(super) fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let mut collection = SettingsCollection::new(
        ID,
        "Sandbox status and repair",
        SettingsGroup::Advanced,
        Vec::new(),
    );
    if cfg!(windows) {
        collection.actions.push(
            SettingsAction::record("setup", "Set up or repair elevated sandbox (UAC)").confirmed(),
        );
    }
    let mut record = record_of("system", "System sandbox", &serde_json::Value::Null, &[]);
    match service.sandbox_status() {
        Ok(SandboxStatus::Available { backend }) => {
            record.state = Some("ready".into());
            record.summary = backend.to_string();
        }
        Ok(SandboxStatus::Unavailable { reason }) => {
            record.state = Some("unavailable".into());
            record.issue = Some(reason);
        }
        Err(error) => record.issue = Some(error.to_string()),
    }
    collection.records.push(record);
    collection
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    if request.action != "setup" || !cfg!(windows) {
        return request.unknown_action();
    }
    service.setup_default_windows_sandbox().await?;
    Ok(SettingsActionOutcome {
        record: None,
        message: Some("Windows sandbox ready; elevated sandbox and workspace-write saved".into()),
    })
}

impl AgentApplicationService {
    /// Provision off the UI thread; configuration changes only after UAC succeeds.
    pub async fn setup_default_windows_sandbox(&self) -> Result<(), String> {
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "sandbox setup worker",
            hanafubuki_agent_sandbox::setup_windows_sandbox,
        )
        .await??;
        let changes = [
            ("windows.sandbox", "elevated"),
            ("sandboxMode", "workspace-write"),
        ]
        .into_iter()
        .map(|(key, value)| SettingChange {
            key: key.into(),
            op: SettingWriteOp::Set(value.into()),
        })
        .collect();
        let outcome = self.write_setting_values(SettingsWrite { changes }).await?;
        if !outcome.shadowed.is_empty() {
            return Err("Sandbox installed, but a higher configuration layer overrides its settings; inspect /config windows".into());
        }
        Ok(())
    }
}
