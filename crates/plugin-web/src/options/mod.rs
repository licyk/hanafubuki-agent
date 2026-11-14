// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! Web plugin settings at the configuration boundary. Providers and credentials
//! are host settings; model arguments cannot change endpoints or key rings.

use hanafubuki_agent_foundation::settings::{
    SettingChoice, SettingKind, SettingSpec, SettingValue,
};
use hanafubuki_agent_web_search::{EngineConfig, SearchOptions, WebSearchProvider as E};
use serde_json::Value;
mod research;

pub fn web_option_schema() -> Vec<SettingSpec> {
    let defaults = SearchOptions::default();
    let mut fields = Vec::new();
    fields.push((
        "provider".into(),
        "Search mode or engine".into(),
        SettingKind::Choice {
            values: [E::Auto, E::Multi]
                .into_iter()
                .chain(E::ALL.iter().copied())
                .map(|e| SettingChoice::new(e.id(), e.id()))
                .collect(),
        },
        SettingValue::Text("auto".into()),
    ));
    for (key, title, values) in [
        ("allowedEngines", "Only use these engines", Vec::new()),
        ("disabledEngines", "Disabled engines", Vec::new()),
        ("fallbackOrder", "Fallback priority", Vec::new()),
        (
            "zhPool",
            "CJK engine pool",
            defaults
                .zh_pool
                .iter()
                .map(|e| e.id().to_string())
                .collect(),
        ),
        (
            "enPool",
            "Other-language engine pool",
            defaults
                .en_pool
                .iter()
                .map(|e| e.id().to_string())
                .collect(),
        ),
        ("blockedDomains", "Blocked result domains", Vec::new()),
        ("disabledTools", "Disabled web tools", Vec::new()),
        (
            "fallbackOn",
            "Failures allowing fallback",
            vec![
                "region",
                "quota",
                "auth",
                "bot-wall",
                "transient",
                "invalid-response",
                "unknown",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        ),
    ] {
        fields.push((
            key.into(),
            title.into(),
            SettingKind::TextList {
                maximum_items: Some(100),
                maximum_item_chars: Some(256),
            },
            SettingValue::TextList(values),
        ));
    }
    for (key, title, value) in [
        ("language", "Search language", "zh"),
        ("bingMarket", "Bing market", ""),
    ] {
        fields.push((
            key.into(),
            title.into(),
            text(false),
            SettingValue::Text(value.into()),
        ));
    }
    fields.push((
        "safeSearch".into(),
        "Safe search".into(),
        SettingKind::Choice {
            values: ["off", "moderate", "strict"]
                .map(|s| SettingChoice::new(s, s))
                .into(),
        },
        SettingValue::Text("off".into()),
    ));
    for (key, title, value, max) in [
        (
            "multiMaxEngines",
            "Maximum parallel engines (0: whole pool)",
            0,
            39,
        ),
        ("multiMinSources", "Target successful engines", 1, 39),
        ("multiMaxResults", "Maximum merged results", 20, 50),
    ] {
        fields.push((
            key.into(),
            title.into(),
            SettingKind::Integer {
                minimum: Some(0),
                maximum: Some(max),
            },
            SettingValue::Integer(value),
        ));
    }
    fields.push((
        "cacheEnabled".into(),
        "Cache search results".into(),
        SettingKind::Boolean,
        SettingValue::Boolean(true),
    ));
    for engine in E::ALL {
        for (suffix, title, secret) in [
            (
                "ApiKeys",
                "API keys (comma, semicolon or newline separated)",
                true,
            ),
            ("Endpoint", "Endpoint override", false),
            ("Model", "Search model", false),
        ] {
            if suffix == "ApiKeys" && engine.key_environment().is_empty() {
                continue;
            }
            if suffix == "Model"
                && !matches!(
                    engine,
                    E::Openai
                        | E::Gemini
                        | E::Claude
                        | E::DeepseekOfficial
                        | E::Perplexity
                        | E::Agy
                )
            {
                continue;
            }
            fields.push((
                format!("{}{suffix}", prefix(*engine)),
                format!("{}: {title}", engine.id()),
                text(secret),
                SettingValue::Text(String::new()),
            ));
        }
    }
    for (key, title) in [
        ("searxngInstances", "SearXNG instances"),
        ("degoogInstances", "Degoog instances"),
    ] {
        fields.push((
            key.into(),
            title.into(),
            SettingKind::TextList {
                maximum_items: Some(20),
                maximum_item_chars: Some(2048),
            },
            SettingValue::TextList(Vec::new()),
        ));
    }
    for (key, title, value) in [
        ("googleCseId", "Google programmable search engine ID", ""),
        ("metasoScope", "Metaso scope", "webpage"),
        ("yandexFolderId", "Yandex folder ID", ""),
        ("yandexSearchType", "Yandex search type", "auto"),
        ("yuanbaoSite", "Yuanbao site", " "),
    ] {
        fields.push((
            key.into(),
            title.into(),
            text(false),
            SettingValue::Text(value.trim().into()),
        ));
    }
    fields.push((
        "yuanbaoMode".into(),
        "Yuanbao mode".into(),
        SettingKind::Integer {
            minimum: Some(0),
            maximum: Some(2),
        },
        SettingValue::Integer(0),
    ));
    let mut specs: Vec<SettingSpec> = fields
        .into_iter()
        .map(
            |(key, title, kind, value): (String, String, SettingKind, SettingValue)| SettingSpec {
                default: Some(value),
                ..SettingSpec::new(key, title, kind)
            },
        )
        .collect();
    specs.extend(research::fields());
    specs
}

pub fn search_options_from_value(mut value: Value) -> Result<SearchOptions, String> {
    let object = value
        .as_object_mut()
        .ok_or("web options must be an object")?;
    for field in [
        "allowedEngines",
        "disabledEngines",
        "fallbackOrder",
        "zhPool",
        "enPool",
    ] {
        if let Some(values) = object.get_mut(field).and_then(Value::as_array_mut) {
            values.retain(|v| {
                v.as_str()
                    .is_some_and(|id| E::ALL.iter().any(|e| e.id() == id))
            });
        }
    }
    if let Some(values) = object.get_mut("platforms").and_then(Value::as_array_mut) {
        values.retain(|v| {
            v.as_str().is_some_and(|id| {
                hanafubuki_agent_web_search::Platform::ALL
                    .iter()
                    .any(|p| p.id() == id)
            })
        });
    }
    let mut options: SearchOptions =
        serde_json::from_value(value.clone()).map_err(|_| "invalid web search options")?;
    let object = value.as_object().ok_or("web options must be an object")?;
    let string = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    for engine in E::ALL {
        let prefix = prefix(*engine);
        let endpoint = string(&format!("{prefix}Endpoint"));
        if !endpoint.is_empty() {
            hanafubuki_agent_foundation::url_policy::UrlPolicy::ENDPOINT
                .parse(&endpoint)
                .map_err(|_| {
                    format!(
                        "{} endpoint must be an HTTP(S) URL without credentials",
                        engine.id()
                    )
                })?;
        }
        let model = string(&format!("{prefix}Model"));
        let keys = string(&format!("{prefix}ApiKeys"));
        let mut config = EngineConfig {
            api_keys: if keys.is_empty() {
                Vec::new()
            } else {
                vec![keys]
            },
            endpoint: (!endpoint.is_empty()).then_some(endpoint),
            model: (!model.is_empty()).then_some(model),
            ..EngineConfig::default()
        };
        if matches!(engine, E::Searxng | E::Degoog) {
            config.instances = object
                .get(&format!("{prefix}Instances"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect();
            for endpoint in &config.instances {
                hanafubuki_agent_foundation::url_policy::UrlPolicy::ENDPOINT
                    .parse(endpoint)
                    .map_err(|_| "invalid search instance endpoint")?;
            }
        }
        config.google_cse_id = string("googleCseId");
        config.metaso_scope = string("metasoScope");
        config.yandex_folder_id = string("yandexFolderId");
        config.yandex_search_type = string("yandexSearchType");
        config.yuanbao_site = string("yuanbaoSite");
        config.yuanbao_mode = object
            .get("yuanbaoMode")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(2) as u8;
        options.engines.insert(*engine, config);
    }
    research::decode(&value, &mut options)?;
    Ok(options)
}

fn text(secret: bool) -> SettingKind {
    SettingKind::Text {
        maximum_chars: Some(16384),
        secret,
        nullable: false,
    }
}
fn prefix(engine: E) -> String {
    let mut upper = false;
    engine
        .id()
        .chars()
        .filter_map(|c| {
            if c == '-' || c == '_' {
                upper = true;
                None
            } else if upper {
                upper = false;
                Some(c.to_ascii_uppercase())
            } else {
                Some(c)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
