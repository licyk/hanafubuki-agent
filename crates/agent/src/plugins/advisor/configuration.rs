use super::{advisor_schema, AdvisorService};
use hanafubuki_agent_config::{SettingChange, SettingWriteOp, SettingsWrite};
use hanafubuki_agent_contracts::AgentSessionId;
use hanafubuki_agent_kernel::AgentError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Serialize)]
pub struct AdvisorProposal {
    pub id: String,
    pub changes: Vec<AdvisorSettingDiff>,
}
#[derive(Clone, Serialize)]
pub struct AdvisorSettingDiff {
    pub key: String,
    pub before: Value,
    pub after: Value,
}
#[derive(Deserialize)]
pub(super) struct ConfigRequest {
    #[serde(default)]
    changes: BTreeMap<String, Value>,
}
impl AdvisorService {
    pub(super) fn config(
        &self,
        id: AgentSessionId,
        request: ConfigRequest,
    ) -> Result<Value, String> {
        let options = self.options.read();
        let current = serde_json::json!(&**options);
        if request.changes.is_empty() {
            return Ok(current);
        }
        let schema = advisor_schema();
        let mut changes = Vec::new();
        for (key, value) in request.changes {
            if matches!(
                key.as_str(),
                "active"
                    | "activationScope"
                    | "provider"
                    | "model"
                    | "fallbackProvider"
                    | "fallbackModel"
                    | "executorProvider"
                    | "executorModel"
            ) {
                return Err(format!(
                    "{key} can only be set by the user in Advisor settings"
                ));
            }
            let spec = schema
                .options
                .iter()
                .find(|s| s.key == key)
                .ok_or_else(|| format!("Unknown Advisor setting {key}"))?;
            let checked = spec.check(&value)?.to_json();
            changes.push(AdvisorSettingDiff {
                key: key.clone(),
                before: current[&key].clone(),
                after: checked,
            });
        }
        let proposal = AdvisorProposal {
            id: super::follow_up::next_id(),
            changes,
        };
        let result = serde_json::json!({"pendingUserConfirmation":proposal,"review":"Use /advisor-settings review to inspect and confirm these exact changes."});
        self.proposals.lock().insert(id, proposal);
        Ok(result)
    }
    pub(crate) fn proposal(&self, id: AgentSessionId) -> Result<AdvisorProposal, AgentError> {
        self.proposals
            .lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| AgentError::InvalidInput("No pending Advisor settings proposal".into()))
    }
    pub(crate) fn resolve_proposal(
        &self,
        id: AgentSessionId,
        key: &str,
        accept: bool,
    ) -> Result<Option<SettingsWrite>, AgentError> {
        let mut pending = self.proposals.lock();
        if !pending.get(&id).is_some_and(|p| p.id == key) {
            return Err(AgentError::InvalidInput(
                "Advisor proposal changed or expired; review it again".into(),
            ));
        }
        Ok(pending
            .remove(&id)
            .filter(|_| accept)
            .map(|p| SettingsWrite {
                changes: p
                    .changes
                    .into_iter()
                    .map(|c| SettingChange {
                        key: format!("pluginOptions.advisor.{}", c.key),
                        op: SettingWriteOp::Set(c.after),
                    })
                    .collect(),
            }))
    }
}
