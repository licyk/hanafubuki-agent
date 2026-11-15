//! Explicit user/project targets alongside the profile collection.
use super::record::record_of;
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, SAVE,
};
use crate::service::AgentApplicationService;
use hanafubuki_agent_config::{ConfigLayerSource, SettingsGroup, SettingsLayer};

pub(super) const ID: &str = "configLayers";

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let fields = super::profiles::fields(service).await;
    let mut collection =
        SettingsCollection::new(ID, "Configuration layers", SettingsGroup::Advanced, fields)
            .with_actions(vec![
                SettingsAction::record(SAVE, "Save to this layer"),
                SettingsAction::record("source", "Show save path and precedence"),
            ]);
    let mut sources = match service.config.layer_sources() {
        Ok(sources) => sources,
        Err(error) => return collection.unavailable(error),
    };
    sources.retain(|source| !matches!(source, ConfigLayerSource::Project { .. }));
    match service.config.project_configuration_source() {
        Ok(Some(source)) => sources.push(source),
        Ok(None) => {}
        Err(error) => return collection.unavailable(error),
    }
    for source in sources {
        let (id, label, layer, trusted, path) = match source {
            ConfigLayerSource::User { path, .. } => (
                "user",
                "User configuration",
                SettingsLayer::User,
                true,
                path,
            ),
            ConfigLayerSource::Project { path, trusted } => (
                "project",
                "Current project configuration",
                SettingsLayer::Project,
                trusted,
                path,
            ),
            _ => continue,
        };
        let mut record = record_of(id, label, &serde_json::Value::Null, &collection.record);
        record.summary = format!(
            "{} · precedence: user < selected profile < trusted project < command line",
            path.display()
        );
        if trusted {
            match service.config.layer_document(&layer) {
                Ok((_, document)) => {
                    let values = record_of(id, label, &document, &collection.record);
                    record.issue = values.issue;
                    record.values = values.values;
                    record.secrets_set = values.secrets_set;
                }
                Err(error) => {
                    record.read_only = Some(error.clone());
                    record.issue = Some(error);
                }
            }
        } else {
            record.read_only = Some(
                "Trust this project in Projects before reading or editing its configuration".into(),
            );
        }
        collection.records.push(record);
    }
    collection
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let layer = match request.record_id()? {
        "user" => SettingsLayer::User,
        "project" => SettingsLayer::Project,
        _ => return Err("Unknown configuration layer".into()),
    };
    let message = match request.action.as_str() {
        SAVE => {
            super::layer_fields::write(
                service,
                layer,
                request.values,
                super::profiles::fields(service).await,
            )
            .await?
        }
        "source" => {
            let (path, _) = service.config.layer_document(&layer)?;
            format!("{layer:?}: {}\nPrecedence: user < selected profile < trusted project < command line. Profiles are edited in Profiles.", path.display())
        }
        _ => return request.unknown_action(),
    };
    Ok(SettingsActionOutcome {
        record: None,
        message: Some(message),
    })
}
