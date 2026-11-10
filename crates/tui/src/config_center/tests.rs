//! The configuration center's coverage contract: every known scalar
//! `config.toml` setting is owned by a form or a specialized editor.

use hanafubuki_agent::{AgentSettings, AgentStandaloneSettings};
use serde_json::Value;

/// Keys the center's own forms edit. Specialized editors own the removed
/// dynamic tables below, not the generic settings patch.
const FORM_KEYS: [&str; 43] = [
    "permissionMode",
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
