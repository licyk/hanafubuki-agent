// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The configuration center's coverage contract: every known scalar
//! `config.toml` setting is owned by a form or a specialized editor, and
//! every domain opens from its key.

use hanafubuki_agent::{AgentSettings, AgentStandaloneSettings};
use serde_json::Value;

/// Keys the center's own forms edit. Specialized editors own the removed
/// dynamic tables below, not the generic settings patch.
const FORM_KEYS: [&str; 44] = [
    "permissionMode",
    "modelMetadata.discovery",
    "sandboxMode",
    "sandboxWorkspaceWrite.writableRoots",
    "sandboxWorkspaceWrite.networkAccess",
    "windows.sandbox",
    "memory.enabled",
    "memory.useMemories",
    "memory.generateMemories",
    "memory.excludeExternalContext",
    "memory.extractionModel",
    "memory.consolidationModel",
    "memory.maxSourceAgeDays",
    "memory.minimumIdleHours",
    "memory.maxSessionsPerPass",
    "memory.maxStage1Outputs",
    "memory.maxUnusedDays",
    "webSearchProvider",
    "searxngBaseUrl",
    "webFetchUseJinaReader",
    "compaction.strategy",
    "compaction.background.thresholdPercent",
    "compaction.background.childSessions",
    "compaction.hierarchical.maxContextLimitPercent",
    "compaction.hierarchical.emergencyThresholdPercent",
    "compaction.hierarchical.nudgeGrowthTokens",
    "compaction.hierarchical.preserveRecentMessages",
    "compaction.hierarchical.preserveRecentTokens",
    "compaction.hierarchical.minCompressRangeChars",
    "compaction.hierarchical.minSummaryLength",
    "compaction.hierarchical.maxSummaryLength",
    "compaction.hierarchical.tiers",
    "compaction.hierarchical.protectedTools",
    "compaction.hierarchical.messageTimestamps",
    "compaction.hierarchical.absorb",
    "compaction.hierarchical.contentStore",
    "compaction.hierarchical.crush",
    "compaction.hierarchical.rules",
    "network.proxy",
    "logLevel",
    "tui.fullscreenTranscript",
    "tui.alternateScreen",
    "tui.rawOutput",
    "tui.statusLineColors",
];

/// Owned by `/plugins`: plugin and package lifecycle calls, not a generic
/// patch, write these dynamic records.
const PLUGIN_KEYS: [&str; 3] = ["disabledPlugins", "plugins", "marketplaces"];

/// Owned by the terminal interface's specialized editors.
const INTERFACE_KEYS: [&str; 5] = [
    "tui.theme",
    "tui.statusLine",
    "tui.terminalTitle",
    "tui.keymap",
    "tui.presets",
];

#[test]
fn every_known_scalar_setting_has_an_editor() {
    let agent = serde_json::to_value(AgentSettings::default()).unwrap();
    let standalone = serde_json::to_value(AgentStandaloneSettings::default()).unwrap();
    let mut leaves = Vec::new();
    leaves_of(&agent, "", &mut leaves);
    leaves_of(&standalone, "", &mut leaves);
    leaves.sort_unstable();
    leaves.dedup();
    leaves.retain(|leaf| {
        !PLUGIN_KEYS.contains(&leaf.as_str())
            && !leaf.starts_with("plugins.")
            && !leaf.starts_with("marketplaces.")
            && !INTERFACE_KEYS.contains(&leaf.as_str())
    });

    let mut covered = FORM_KEYS;
    covered.sort_unstable();
    assert_eq!(leaves, covered);
}

fn leaves_of(value: &Value, prefix: &str, leaves: &mut Vec<String>) {
    match value {
        Value::Object(table) => {
            for (key, value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaves_of(value, &path, leaves);
            }
        }
        Value::Array(_) | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            leaves.push(prefix.to_string());
        }
        Value::Null => leaves.push(prefix.to_string()),
    }
}

/// The generic editor covers every option of every descriptor: each has
/// its field, an untouched form saves nothing, and an edited field saves
/// what the write checks.
#[test]
fn every_option_of_every_descriptor_has_the_generic_editor() {
    use hanafubuki_agent::{agent_plugin_descriptors, ResolvedPluginOption};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut described = 0;
    for descriptor in agent_plugin_descriptors() {
        let Some(schema) = descriptor.options else {
            continue;
        };
        let values = schema
            .options
            .iter()
            .map(|option| ResolvedPluginOption {
                key: option.key.clone(),
                value: option.default.clone(),
                layer: None,
            })
            .collect::<Vec<_>>();
        let mut form = super::options_form(descriptor.name, &schema, &values);
        for index in 0..schema.options.len() {
            let id = super::options::option_field(index);
            form.set_error(id, "probe");
            assert_eq!(
                form.error(id),
                Some("probe"),
                "{}",
                schema.options[index].key
            );
            described += 1;
        }
        assert_eq!(
            super::changed_options(&mut form, &schema, &values),
            Some(serde_json::Map::new())
        );
    }
    assert_eq!(described, 4, "the subagents options");

    let schema = agent_plugin_descriptors()
        .into_iter()
        .find_map(|descriptor| descriptor.options)
        .unwrap();
    let values = schema
        .options
        .iter()
        .map(|option| ResolvedPluginOption {
            key: option.key.clone(),
            value: option.default.clone(),
            layer: None,
        })
        .collect::<Vec<_>>();
    let mut form = super::options_form("subagents", &schema, &values);
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    form.handle_key(key(KeyCode::Backspace));
    form.handle_key(key(KeyCode::Char('5')));
    assert_eq!(
        super::changed_options(&mut form, &schema, &values),
        Some(serde_json::Map::from_iter([(
            "maximumRunningAgents".to_string(),
            serde_json::json!(5)
        )]))
    );
    form.handle_key(key(KeyCode::Char('0')));
    assert_eq!(super::changed_options(&mut form, &schema, &values), None);
    assert_eq!(
        form.error(super::options::option_field(0)),
        Some("`maximumRunningAgents` is between 1 and 16")
    );
}

/// Each domain opens from its key; the session's role opens `/role`'s
/// picker, which lists the usable roles and sets the session's between
/// turns.
#[test]
fn every_domain_opens_from_its_key_and_the_role_opens_the_role_picker() {
    use super::{ConfigAction, ConfigCenter, ConfigSection};

    for section in ConfigSection::ALL {
        assert_eq!(ConfigSection::parse(section.key()), Some(section));
    }
    assert_eq!(ConfigSection::parse("roles"), Some(ConfigSection::Role));
    let mut center = ConfigCenter::new(Err("not read".to_string()));
    assert!(matches!(
        center.open(ConfigSection::Role, true),
        Some(ConfigAction::Replay(command)) if command == "/role"
    ));
}
