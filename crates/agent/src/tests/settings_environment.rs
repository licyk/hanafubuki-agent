use super::*;
use hanafubuki_agent_config::{SettingChange, SettingWriteOp};

#[tokio::test(flavor = "multi_thread")]
async fn every_shell_environment_option_is_editable_and_saved_globally() {
    let (service, paths, _) = service_with_config("settings-environment", |_, _| {});
    let values = [
        ("environmentInherit", json!("unfiltered")),
        ("environmentIgnoreDefaultExcludes", json!(false)),
        ("environmentExclude", json!(["PRIVATE_*"])),
        ("environmentIncludeOnly", json!(["PATH", "APP_MODE"])),
        ("environmentSet", json!(["APP_MODE=development"])),
        ("shellUseProfile", json!(true)),
        ("shellSnapshot", json!(false)),
    ];
    let catalog = service.settings_catalog().await.unwrap();
    let section = catalog
        .sections
        .iter()
        .find(|section| section.id == "pluginOptions.execution")
        .unwrap();
    assert_eq!(section.group, SettingsGroup::Plugins);
    for (name, _) in &values {
        let key = format!("pluginOptions.execution.{name}");
        let spec = section
            .settings
            .iter()
            .find(|spec| spec.key == key)
            .unwrap();
        assert!(spec.visible_if.is_none(), "{key} must remain editable");
        if *name == "environmentInherit" {
            assert_eq!(spec.default, Some(SettingValue::Text("all".into())));
        }
    }
    service
        .write_setting_values(SettingsWrite {
            changes: values
                .iter()
                .map(|(name, value)| SettingChange {
                    key: format!("pluginOptions.execution.{name}"),
                    op: SettingWriteOp::Set(value.clone()),
                })
                .collect(),
        })
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let saved = AgentConfig::new(&paths.config_dir).settings().unwrap();
    for (name, value) in &values {
        let key = format!("pluginOptions.execution.{name}");
        let state = catalog
            .sections
            .iter()
            .flat_map(|section| &section.values)
            .find(|state| state.key == key)
            .unwrap();
        assert_eq!(state.source, SettingSource::User);
        assert_eq!(saved.plugin_options.value("execution", name), Some(value));
    }
    service
        .write_setting_values(SettingsWrite {
            changes: vec![SettingChange {
                key: "pluginOptions.execution.environmentInherit".into(),
                op: SettingWriteOp::Set(json!("all")),
            }],
        })
        .await
        .unwrap();
    let saved = AgentConfig::new(&paths.config_dir).settings().unwrap();
    assert_eq!(
        saved
            .plugin_options
            .value("execution", "environmentInherit"),
        Some(&json!("all"))
    );
    assert_eq!(
        saved
            .plugin_options
            .value("execution", "environmentExclude"),
        Some(&json!(["PRIVATE_*"]))
    );
    let _ = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
