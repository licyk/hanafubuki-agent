//! The roles as the Agent last read them: the usable ones with their
//! fields, the unusable ones with why. A role of the user's `config.toml`
//! (`[agents.<name>]`) is added, edited and deleted here, key by key, then
//! the catalog is read again; one of a role file, a profile, a project, a
//! plugin or the Agent itself is shown with where it is written. The keys
//! that narrow a role (`extends`, `permissionModeCap`, `tools`, `plugins`)
//! are written in the file.

use std::collections::BTreeMap;

use hanafubuki_agent_config::{
    RoleLayer, RoleOrigin, SettingChoice, SettingKind, SettingSpec, SettingValue, SettingsGroup,
    UserEntry,
};
use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
use serde_json::{Map, Value};

use super::profiles::user_table;
use super::record::record_of;
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection,
    SettingsRecord, ADD, DELETE, SAVE,
};
use crate::roles::AgentRoleSource;
use crate::service::AgentApplicationService;

pub(super) const ID: &str = "roles";
const AGENTS: &str = "agents";
const REASONING_EFFORT: &str = "reasoningEffort";

fn fields() -> Vec<SettingSpec> {
    let text = SettingKind::Text {
        maximum_chars: None,
        secret: false,
        nullable: true,
    };
    vec![
        SettingSpec::new("description", "Description", text.clone()),
        SettingSpec {
            summary: "Given to the agent that takes the role".into(),
            ..SettingSpec::new("instructions", "Instructions", text.clone())
        },
        SettingSpec {
            summary: "After the instructions of the role it extends, or in their place".into(),
            ..SettingSpec::new(
                "instructionsMode",
                "Instructions mode",
                SettingKind::Choice {
                    values: vec![
                        SettingChoice::new("append", "Append"),
                        SettingChoice::new("replace", "Replace"),
                    ],
                },
            )
        },
        SettingSpec::new("model", "Model", text.clone()),
        SettingSpec {
            summary: "minimal, low, medium, high, xhigh or the model's own level".into(),
            ..SettingSpec::new(REASONING_EFFORT, "Reasoning effort", text)
        },
        SettingSpec {
            summary: "Whether an agent may start another with this role".into(),
            ..SettingSpec::new("spawnable", "Spawnable", SettingKind::Boolean)
        },
    ]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let catalog = service.roles();
    let collection = SettingsCollection::new(ID, "Roles", SettingsGroup::Model, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a role").named("Role name"),
            SettingsAction::record(SAVE, "Save"),
            SettingsAction::record(DELETE, "Delete").confirmed(),
        ]);
    let entries = match user_table(service, AGENTS).await {
        Ok(entries) => entries.into_iter().collect::<BTreeMap<_, _>>(),
        Err(error) => return collection.unavailable(error),
    };
    let fields = fields();
    let text = |value: &str| SettingValue::Text(value.to_string());
    let usable = catalog.roles().map(|role| {
        let mut record = match entries.get(&role.name).filter(|_| is_users(&role.source)) {
            Some(entry) => record_of(&role.name, &role.name, entry, &fields),
            None => {
                let mut values = BTreeMap::from([
                    ("description".to_string(), text(&role.description)),
                    (
                        "spawnable".to_string(),
                        SettingValue::Boolean(role.spawnable),
                    ),
                ]);
                if let Some(model) = &role.model {
                    values.insert("model".to_string(), text(model));
                }
                if let Some(effort) = &role.reasoning_effort {
                    values.insert(REASONING_EFFORT.to_string(), text(effort.as_str()));
                }
                SettingsRecord {
                    values,
                    ..record_of(&role.name, &role.name, &Value::Null, &fields)
                }
            }
        };
        record.summary = role.source.to_string();
        record.read_only = read_only(&role.source);
        record
    });
    let unusable = catalog.unusable().map(|role| {
        let mut record = match entries.get(&role.name).filter(|_| is_users(&role.source)) {
            Some(entry) => record_of(&role.name, &role.name, entry, &fields),
            None => record_of(&role.name, &role.name, &Value::Null, &fields),
        };
        record.summary = role.source.to_string();
        record.state = Some("unusable".to_string());
        record.issue = Some(role.reason.clone());
        record.read_only = read_only(&role.source);
        record
    });
    SettingsCollection {
        records: usable.chain(unusable).collect(),
        issues: catalog.issues().iter().map(ToString::to_string).collect(),
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let name = request.record_id()?.to_string();
    let catalog = service.roles();
    let source = catalog
        .role(&name)
        .map(|role| &role.source)
        .or_else(|| catalog.unusable_role(&name).map(|role| &role.source));
    match (request.action.as_str(), source) {
        (ADD, Some(source)) if is_users(source) => {
            return Err(format!("the role `{name}` exists"));
        }
        (SAVE | DELETE, Some(source)) => {
            if let Some(reason) = read_only(source) {
                return Err(reason);
            }
        }
        (SAVE | DELETE, None) => return Err(format!("there is no role `{name}`")),
        (ADD, _) if request.values.values().all(Value::is_null) => {
            return Err("a role is added with at least one of its keys".to_string());
        }
        (ADD, _) => {}
        _ => return request.unknown_action(),
    }
    let values = checked(&request.values)?;
    let (config, roles, packages) = (
        service.config.clone(),
        service.roles.clone(),
        service.packages.clone(),
    );
    let delete = request.action == DELETE;
    let written = name.clone();
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent role write worker",
        move || {
            if delete {
                config.remove_entry(UserEntry::Role(&written))?;
            } else {
                config.write_entry_values(UserEntry::Role(&written), &values)?;
            }
            roles.rebuild(&config, || packages.loaded().roles());
            Ok::<_, String>(())
        },
    )
    .await??;
    Ok(SettingsActionOutcome {
        record: (request.action == ADD).then_some(name),
        message: None,
    })
}

/// The values a write takes, each checked by its field's spec (a
/// reasoning effort as a level), `null` removing the key.
fn checked(values: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let fields = fields();
    values
        .iter()
        .map(|(key, value)| {
            let spec = fields
                .iter()
                .find(|spec| spec.key == *key)
                .ok_or_else(|| format!("`{key}` is not a field of a role written here"))?;
            let value = match value {
                Value::Null => Value::Null,
                value => spec.check(value)?.to_json(),
            };
            if let (REASONING_EFFORT, Value::String(effort)) = (key.as_str(), &value) {
                ModelReasoningEffort::parse(effort.as_str())?;
            }
            Ok((key.clone(), value))
        })
        .collect()
}

/// Whether the role is an entry of the user's `config.toml`.
fn is_users(source: &AgentRoleSource) -> bool {
    matches!(
        source,
        AgentRoleSource::Configured {
            layer: RoleLayer::User,
            origin: RoleOrigin::Table {
                config_file: None,
                ..
            },
        }
    )
}

/// Why a role is not written here, when it is not.
fn read_only(source: &AgentRoleSource) -> Option<String> {
    match source {
        _ if is_users(source) => None,
        AgentRoleSource::BuiltIn => {
            Some("A built-in role; a role of the same name replaces it".into())
        }
        AgentRoleSource::Configured {
            origin:
                RoleOrigin::Table {
                    config_file: Some(file),
                    ..
                },
            ..
        } => Some(format!("Written in its role file {}", file.display())),
        AgentRoleSource::Configured { origin, .. } | AgentRoleSource::Package { origin, .. } => {
            Some(format!("Written in {origin}"))
        }
    }
}
