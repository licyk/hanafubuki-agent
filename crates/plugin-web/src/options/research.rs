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

use super::*;

pub(super) fn fields() -> Vec<SettingSpec> {
    let defaults = SearchOptions::default();
    let mut fields = Vec::new();
    for (key, title, value, secret) in [
        (
            "rerankKey",
            "SiliconFlow reranking and embedding API key",
            "",
            true,
        ),
        (
            "rerankEndpoint",
            "Reranking and embedding endpoint",
            defaults.research.rerank_endpoint.as_str(),
            false,
        ),
        ("decisionKey", "Research decision API key", "", true),
        (
            "decisionEndpoint",
            "Research decision endpoint (optional)",
            "",
            false,
        ),
        (
            "fetchExaEndpoint",
            "Exa page extraction endpoint",
            defaults.fetch.exa_endpoint.as_str(),
            false,
        ),
        (
            "fetchFirecrawlEndpoint",
            "Firecrawl page extraction endpoint",
            defaults.fetch.firecrawl_endpoint.as_str(),
            false,
        ),
        (
            "fetchCrawl4aiEndpoint",
            "Crawl4AI page extraction endpoint",
            defaults.fetch.crawl4ai_endpoint.as_str(),
            false,
        ),
        ("crawl4aiApiKey", "Crawl4AI API key", "", true),
    ] {
        fields.push(SettingSpec {
            default: Some(SettingValue::Text(value.into())),
            ..SettingSpec::new(key, title, text(secret))
        });
    }
    for (key, title, pool) in [
        (
            "researchPrimaryPool",
            "Research first-round engines",
            defaults.research.primary_pool,
        ),
        (
            "researchEscalationPool",
            "Research follow-up engines",
            defaults.research.escalation_pool,
        ),
    ] {
        fields.push(SettingSpec {
            default: Some(SettingValue::TextList(
                pool.iter().map(|e| e.id().into()).collect(),
            )),
            ..SettingSpec::new(
                key,
                title,
                SettingKind::TextList {
                    maximum_items: Some(39),
                    maximum_item_chars: Some(32),
                },
            )
        });
    }
    fields.push(SettingSpec {
        default: Some(SettingValue::Text("exa".into())),
        ..SettingSpec::new(
            "fetchPrefer",
            "Preferred page extraction fallback",
            SettingKind::Choice {
                values: ["exa", "firecrawl"]
                    .map(|s| SettingChoice::new(s, s))
                    .into(),
            },
        )
    });
    fields.push(SettingSpec {
        default: Some(SettingValue::Boolean(false)),
        ..SettingSpec::new(
            "localHelpersEnabled",
            "Enable installed AGY/Grok/browser helpers (requires sandbox escalation approval)",
            SettingKind::Boolean,
        )
    });
    fields.extend(extra_fields());
    fields
}

pub(super) fn decode(value: &Value, options: &mut SearchOptions) -> Result<(), String> {
    for (key, target) in [
        ("rerankKey", &mut options.research.rerank_key),
        ("rerankEndpoint", &mut options.research.rerank_endpoint),
        ("decisionKey", &mut options.research.decision_key),
        ("decisionEndpoint", &mut options.research.decision_endpoint),
        ("fetchExaEndpoint", &mut options.fetch.exa_endpoint),
        (
            "fetchFirecrawlEndpoint",
            &mut options.fetch.firecrawl_endpoint,
        ),
        (
            "fetchCrawl4aiEndpoint",
            &mut options.fetch.crawl4ai_endpoint,
        ),
        ("crawl4aiApiKey", &mut options.fetch.crawl4ai_api_key),
    ] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if key.ends_with("Endpoint") && !text.trim().is_empty() {
                hanafubuki_agent_foundation::url_policy::UrlPolicy::ENDPOINT
                    .parse(text.trim())
                    .map_err(|_| {
                        format!("{key} must be an HTTP(S) endpoint without credentials")
                    })?;
            }
            *target = text.trim().into();
        }
    }
    for (key, target) in [
        ("researchPrimaryPool", &mut options.research.primary_pool),
        (
            "researchEscalationPool",
            &mut options.research.escalation_pool,
        ),
    ] {
        if let Some(values) = value.get(key).and_then(Value::as_array) {
            *target = values
                .iter()
                .filter_map(|v| v.as_str())
                .filter_map(|id| E::ALL.iter().find(|e| e.id() == id).copied())
                .collect();
        }
    }
    if let Some(value) = value.get("fetchPrefer") {
        options.fetch.prefer =
            serde_json::from_value(value.clone()).map_err(|_| "invalid fetch preference")?;
    }
    if let Some(value) = value.get("browserFetch").and_then(Value::as_bool) {
        options.fetch.browser_enabled = value;
    }
    if let Some(values) = value.get("allowFakeIpCidrs").and_then(Value::as_array) {
        options.fetch.allow_fake_ip_cidrs = values
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
    }
    if let Some(values) = value.get("platforms").and_then(Value::as_array) {
        options.platforms = values
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
    }
    Ok(())
}

fn extra_fields() -> Vec<SettingSpec> {
    vec![
        SettingSpec {
            default: Some(SettingValue::Boolean(true)),
            ..SettingSpec::new(
                "browserFetch",
                "Use an installed browser for difficult pages",
                SettingKind::Boolean,
            )
        },
        SettingSpec {
            default: Some(SettingValue::Integer(5)),
            ..SettingSpec::new(
                "cacheTtlMinutes",
                "Search cache lifetime in minutes",
                SettingKind::Integer {
                    minimum: Some(0),
                    maximum: Some(5),
                },
            )
        },
        SettingSpec {
            default: Some(SettingValue::Text("en".into())),
            ..SettingSpec::new(
                "promptLanguage",
                "Tool instruction language",
                SettingKind::Choice {
                    values: ["en", "zh"].map(|s| SettingChoice::new(s, s)).into(),
                },
            )
        },
        SettingSpec {
            default: Some(SettingValue::Null),
            ..SettingSpec::new(
                "region",
                "Search region",
                SettingKind::Text {
                    maximum_chars: Some(16),
                    secret: false,
                    nullable: true,
                },
            )
        },
        SettingSpec {
            default: Some(SettingValue::TextList(Vec::new())),
            ..SettingSpec::new(
                "allowFakeIpCidrs",
                "Reserved DNS ranges used by a transparent proxy",
                SettingKind::TextList {
                    maximum_items: Some(20),
                    maximum_item_chars: Some(64),
                },
            )
        },
        SettingSpec {
            default: Some(SettingValue::TextList(
                hanafubuki_agent_web_search::Platform::ALL
                    .iter()
                    .map(|p| p.id().into())
                    .collect(),
            )),
            ..SettingSpec::new(
                "platforms",
                "Enabled native search platforms",
                SettingKind::TextList {
                    maximum_items: Some(14),
                    maximum_item_chars: Some(32),
                },
            )
        },
    ]
}
