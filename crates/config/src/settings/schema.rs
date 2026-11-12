use serde_json::{json, Value};

use super::AgentWebSearchProvider;

/// The settings page's form for the web tools, in the settings schema's
/// field shape: the editor submits a field as it settles (`commit`), and
/// submits the members of a `group` together, since `searxngBaseUrl` is
/// required when `webSearchProvider` is `searxng`.
pub fn agent_settings_fields() -> Value {
    let options = AgentWebSearchProvider::ALL
        .iter()
        .map(|provider| {
            json!({
                "value": provider.as_str(),
                "labelKey": format!("settings.options.webSearchProvider.{}", provider.as_str()),
            })
        })
        .collect::<Vec<_>>();
    json!([
        {"key": "webSearchProvider", "labelKey": "settings.fields.webSearchProvider.label", "type": "select",
         "commit": "change", "group": "webSearch", "options": options, "helpKey": "settings.fields.webSearchProvider.help"},
        {"key": "searxngBaseUrl", "labelKey": "settings.fields.searxngBaseUrl.label", "type": "string", "nullable": true,
         "commit": "blur", "group": "webSearch",
         "helpKey": "settings.fields.searxngBaseUrl.help",
         "disabledWhen": {"key": "webSearchProvider", "equals": "duckduckgo"},
         "disabledReasonKey": "settings.fields.searxngBaseUrl.duckduckgoOnly"},
        {"key": "webFetchUseJinaReader", "labelKey": "settings.fields.webFetchUseJinaReader.label", "type": "boolean",
         "commit": "change",
         "helpKey": "settings.fields.webFetchUseJinaReader.help"},
        {"key": "webFetchResolveThroughProxy", "labelKey": "settings.fields.webFetchResolveThroughProxy.label", "type": "boolean",
         "commit": "change",
         "helpKey": "settings.fields.webFetchResolveThroughProxy.help"}
    ])
}
