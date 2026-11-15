//! Typed authoring for user role files, with other sources kept read-only.
use super::record::record_of;
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD, SAVE,
};
use crate::roles::AgentRoleSource;
use crate::service::AgentApplicationService;
use hanafubuki_agent_config::{RoleLayer, RoleOrigin, SettingsGroup};

pub(super) const ID: &str = "roleFiles";

pub(super) fn fields() -> Vec<hanafubuki_agent_config::SettingSpec> {
    super::roles::fields()
        .into_iter()
        .filter(|field| field.key != "configFile")
        .collect()
}

pub(super) fn path(source: &AgentRoleSource) -> Option<&std::path::Path> {
    match source {
        AgentRoleSource::Configured { origin, .. } | AgentRoleSource::Package { origin, .. } => {
            match origin {
                RoleOrigin::File { path }
                | RoleOrigin::Table {
                    config_file: Some(path),
                    ..
                } => Some(path),
                _ => None,
            }
        }
        _ => None,
    }
}

pub(super) fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let mut collection =
        SettingsCollection::new(ID, "Role source files", SettingsGroup::Model, fields())
            .with_actions(vec![
                SettingsAction::collection(ADD, "Create a user role file").named("Role name"),
                SettingsAction::record(SAVE, "Save role file"),
                SettingsAction::record("source", "Show source path"),
            ]);
    let catalog = service.roles();
    let sources = catalog
        .roles()
        .map(|role| (&role.name, &role.source))
        .chain(catalog.unusable().map(|role| (&role.name, &role.source)));
    for (name, source) in sources {
        let Some(path) = path(source) else {
            continue;
        };
        let mut record = record_of(name, name, &serde_json::Value::Null, &collection.record);
        record.summary = source.to_string();
        match service.config.read_role_document(path) {
            Ok(document) => {
                let values = record_of(name, name, &document, &collection.record);
                record.issue = values.issue;
                record.values = values.values;
                record.secrets_set = values.secrets_set;
            }
            Err(error) => {
                record.issue = Some(error.clone());
                record.read_only = Some(error);
            }
        }
        if !matches!(
            source,
            AgentRoleSource::Configured {
                layer: RoleLayer::User,
                ..
            }
        ) || !service.config.role_file_is_writable(path).unwrap_or(false)
        {
            record.read_only =
                Some("This source is read-only; copy the role to user configuration first".into());
        }
        collection.records.push(record);
    }
    collection
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
    let path = if request.action == ADD {
        if source.is_some() {
            return Err("A role with that name already exists".into());
        }
        let path = service.config.user_role_file(&name)?;
        if path.exists() {
            return Err("That role file already exists".into());
        }
        path
    } else {
        let source = source.ok_or("Role no longer exists")?;
        let path = path(source).ok_or("Role has no source file")?.to_path_buf();
        if request.action == "source" {
            return Ok(SettingsActionOutcome {
                record: None,
                message: Some(source.to_string()),
            });
        }
        if !matches!(
            source,
            AgentRoleSource::Configured {
                layer: RoleLayer::User,
                ..
            }
        ) {
            return Err("Only user-owned role files can be edited here".into());
        }
        path
    };
    if !matches!(request.action.as_str(), ADD | SAVE) {
        return request.unknown_action();
    }
    let values = super::roles::checked(&request.values)?;
    let config = service.config.clone();
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "write role source file",
        move || config.write_user_role_file(&path, &values),
    )
    .await??;
    service
        .roles
        .rebuild(&service.config, || service.packages.loaded().roles());
    Ok(SettingsActionOutcome {
        record: (request.action == ADD).then_some(name),
        message: None,
    })
}
