use std::collections::BTreeMap;

use hanafubuki_agent_config::{
    AgentConfig, PluginOptionSchema, SettingApply, SettingKind, SettingSource, SettingSpec,
    SettingValue, SettingsGroup, SettingsSection,
};
use serde_json::json;

use super::{reset, set, setup, write};

fn state<'a>(
    sections: &'a [SettingsSection],
    key: &str,
) -> &'a hanafubuki_agent_config::SettingState {
    sections
        .iter()
        .flat_map(|section| &section.values)
        .find(|state| state.key == key)
        .unwrap_or_else(|| panic!("no value for {key}"))
}

#[test]
fn each_section_has_its_values_their_layers_and_their_issues() {
    let fixture = setup(
        "[profiles.work]\nwebFetchUseJinaReader = true\n[memory]\nenabled = true\n",
        "[memory]\nmaxUnusedDays = 0\n",
    );
    fixture.trust_project(&fixture.layered(None, &[]));
    let config = fixture.layered(Some("work"), &["logLevel=trace"]);
    let sections = config.settings_sections().unwrap();
    for section in &sections {
        let keys = section.settings.iter().map(|spec| &spec.key);
        assert!(
            keys.eq(section.values.iter().map(|state| &state.key)),
            "{}: one value per setting, in order",
            section.id
        );
    }
    let memory = sections
        .iter()
        .find(|section| section.id == "memory")
        .unwrap();
    assert_eq!(memory.group, SettingsGroup::Memory);

    let enabled = state(&sections, "memory.enabled");
    assert_eq!(enabled.value, Some(SettingValue::Boolean(true)));
    assert_eq!(enabled.source, SettingSource::User);
    let jina = state(&sections, "webFetchUseJinaReader");
    assert_eq!(
        jina.source,
        SettingSource::Profile {
            name: "work".into()
        }
    );
    assert_eq!(state(&sections, "logLevel").source, SettingSource::Override);
    let unused = state(&sections, "memory.maxUnusedDays");
    assert_eq!(unused.value, Some(SettingValue::Integer(30)), "the default");
    assert_eq!(unused.source, SettingSource::Default);
    let issue = unused.issue.as_deref().unwrap();
    assert!(issue.contains("memory.maxUnusedDays"), "{issue}");
    assert_eq!(state(&sections, "searxngBaseUrl").value, None, "unset");
}

#[test]
fn a_hosts_configuration_has_no_standalone_sections() {
    let fixture = setup("", "");
    let ids = AgentConfig::new(&fixture.config_dir)
        .settings_sections()
        .unwrap()
        .into_iter()
        .map(|section| section.id)
        .collect::<Vec<_>>();
    assert!(ids.contains(&"web".to_string()), "{ids:?}");
    assert!(!ids.contains(&"network".to_string()), "{ids:?}");
    assert!(!ids.contains(&"tui".to_string()), "{ids:?}");
    let layered = fixture.layered(None, &[]).settings_sections().unwrap();
    assert!(layered.iter().any(|section| section.id == "tui"));
}

#[test]
fn a_plugins_options_are_written_by_their_schema_with_the_settings() {
    let fixture = setup("", "");
    let config = fixture.layered(None, &[]);
    let options = BTreeMap::from([(
        "demo".to_string(),
        PluginOptionSchema {
            options: vec![SettingSpec {
                apply: SettingApply::NextSession,
                ..SettingSpec::new(
                    "depth",
                    "Depth",
                    SettingKind::Integer {
                        minimum: Some(1),
                        maximum: Some(4),
                    },
                )
            }],
        },
    )]);
    let outcome = config
        .write_values_with_options(
            &write(vec![
                set("pluginOptions.demo.depth", json!(3)),
                set("memory.enabled", json!(true)),
            ]),
            &options,
        )
        .unwrap();
    assert_eq!(
        outcome.apply,
        [
            (
                "pluginOptions.demo.depth".to_string(),
                SettingApply::NextSession
            ),
            ("memory.enabled".to_string(), SettingApply::Live),
        ]
    );
    let file = fixture.user_file();
    assert!(file.contains("[pluginOptions.demo]\ndepth = 3"), "{file}");
    assert_eq!(
        config.setting_sources().unwrap()["pluginOptions.demo.depth"],
        SettingSource::User
    );

    for (change, refusal) in [
        (set("pluginOptions.demo.depth", json!(9)), "between 1 and 4"),
        (
            set("pluginOptions.demo.width", json!(1)),
            "no option `width`",
        ),
        (
            set("pluginOptions.other.depth", json!(1)),
            "no option `depth`",
        ),
    ] {
        let error = config
            .write_values_with_options(&write(vec![change]), &options)
            .unwrap_err();
        assert!(error.contains(refusal), "{error}");
    }
    assert_eq!(fixture.user_file(), file, "a refusal writes nothing");
    let error = config
        .write_values(&write(vec![set("pluginOptions.demo.depth", json!(2))]))
        .unwrap_err();
    assert!(error.contains("no option"), "{error}");

    config
        .write_values_with_options(&write(vec![reset("pluginOptions.demo.depth")]), &options)
        .unwrap();
    assert!(!fixture.user_file().contains("depth"));
}

#[test]
fn turning_a_plugin_off_writes_that_key_alone() {
    let fixture = setup("[memory]\nenabled = true\n", "");
    let config = fixture.layered(None, &[]);
    config.set_plugin_disabled("web", true).unwrap();
    let file = fixture.user_file();
    assert!(file.contains("disabledPlugins = [\"web\"]"), "{file}");
    assert!(!file.contains("permissionMode"), "{file}");
    let sources = config.setting_sources().unwrap();
    assert_eq!(sources["disabledPlugins"], SettingSource::User);
    assert!(!sources.contains_key("permissionMode"), "still the default");
    config.set_plugin_disabled("web", false).unwrap();
    assert!(config.settings().unwrap().disabled_plugins.is_empty());
}
