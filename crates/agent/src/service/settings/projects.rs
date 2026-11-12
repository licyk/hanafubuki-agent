//! The projects of the user's `config.toml` (`[projects."<path>"]`), the
//! standalone Agent's: whether each one's own `config.toml` is read. Only
//! the user's file trusts a project.

use hanafubuki_agent_config::{SettingChoice, SettingKind, SettingSpec, SettingsGroup};
use serde_json::Value;

use super::profiles::{quoted, user_table};
use super::record::record_of;
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD,
};
use crate::service::AgentApplicationService;

pub(super) const ID: &str = "projects";
const PROJECTS: &str = "projects";
const TRUST: &str = "trust";
const UNTRUST: &str = "untrust";
const TRUSTED: &str = "trusted";
const UNTRUSTED: &str = "untrusted";

fn fields() -> Vec<SettingSpec> {
    vec![SettingSpec {
        summary: "Whether the project's own config.toml is read".into(),
        ..SettingSpec::new(
            "trustLevel",
            "Trust",
            SettingKind::Choice {
                values: vec![
                    SettingChoice::new(TRUSTED, "Trusted"),
                    SettingChoice::new(UNTRUSTED, "Not trusted"),
                ],
            },
        )
    }]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "Projects", SettingsGroup::Advanced, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Trust a project"),
            SettingsAction::record(TRUST, "Trust").confirmed(),
            SettingsAction::record(UNTRUST, "Stop trusting"),
        ]);
    let projects = match user_table(service, PROJECTS).await {
        Ok(projects) => projects,
        Err(error) => return collection.unavailable(error),
    };
    let fields = fields();
    let records = projects
        .iter()
        .map(|(path, project)| {
            let mut record = record_of(path, path, project, &fields);
            record.state = project
                .get("trustLevel")
                .and_then(Value::as_str)
                .map(str::to_string);
            record
        })
        .collect();
    SettingsCollection {
        records,
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let level = match request.action.as_str() {
        ADD | TRUST => TRUSTED,
        UNTRUST => UNTRUSTED,
        _ => return request.unknown_action(),
    };
    let path = request.record_id()?.to_string();
    let key = format!("{PROJECTS}.{}.trustLevel", quoted(&path));
    let config = service.config.clone();
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent project trust write worker",
        move || config.set_user_json(&key, Value::from(level)),
    )
    .await??;
    service.apply_settings_to_plugins();
    Ok(SettingsActionOutcome {
        record: (request.action == ADD).then_some(path),
        message: None,
    })
}
