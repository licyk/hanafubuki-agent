//! The web tools: which service searches, and how a page is fetched.

use hanafubuki_agent_foundation::settings::{SettingChoice, SettingKind, SettingSpec};
use hanafubuki_agent_foundation::url_policy::{UrlPolicy, MAX_URL_BYTES};
use serde::{Deserialize, Serialize};

use super::AgentSettings;
use crate::fields::FieldRead;
use crate::sections::{described, SettingsGroup, SettingsRead, SettingsSection};

const WEB_SEARCH_PROVIDER: &str = "webSearchProvider";
const SEARXNG_BASE_URL: &str = "searxngBaseUrl";
/// The provider and its endpoint are written together: SearXNG needs one.
const WEB_SEARCH_GROUP: &str = "webSearch";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWebSearchProvider {
    Auto,
    Duckduckgo,
    Searxng,
}

impl AgentWebSearchProvider {
    pub const ALL: &'static [Self] = &[Self::Auto, Self::Duckduckgo, Self::Searxng];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Duckduckgo => "duckduckgo",
            Self::Searxng => "searxng",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Duckduckgo => "DuckDuckGo",
            Self::Searxng => "SearXNG",
        }
    }
}

pub(crate) fn section() -> SettingsSection {
    let search = |spec: SettingSpec| SettingSpec {
        commit_group: Some(WEB_SEARCH_GROUP.into()),
        keywords: vec!["search".into()],
        ..spec
    };
    SettingsSection::new(
        "web",
        "Web tools",
        SettingsGroup::Tools,
        10,
        vec![
            search(described(
                SettingSpec::new(
                    "webSearchProvider",
                    "Web search provider",
                    SettingKind::Choice {
                        values: AgentWebSearchProvider::ALL
                            .iter()
                            .map(|provider| SettingChoice::new(provider.as_str(), provider.label()))
                            .collect(),
                    },
                ),
                "The web search service",
            )),
            search(SettingSpec {
                help: Some("An http or https URL without credentials, query or fragment".into()),
                ..described(
                    SettingSpec::new(
                        "searxngBaseUrl",
                        "SearXNG URL",
                        SettingKind::Text {
                            maximum_chars: None,
                            secret: false,
                            nullable: true,
                        },
                    ),
                    "Your SearXNG instance; the SearXNG provider needs it",
                )
            }),
            described(
                SettingSpec::new(
                    "webFetchUseJinaReader",
                    "Jina Reader fallback",
                    SettingKind::Boolean,
                ),
                "When a direct fetch fails, read the page through https://r.jina.ai/",
            ),
            SettingSpec {
                advanced: true,
                ..described(
                    SettingSpec::new(
                        "webFetchResolveThroughProxy",
                        "Resolve through the proxy",
                        SettingKind::Boolean,
                    ),
                    "With a proxy URL, leave a name this machine cannot resolve to the proxy instead of failing the fetch",
                )
            },
        ],
    )
}

/// The SearXNG endpoint, normalized; one that cannot be used keeps the
/// layer below.
pub(crate) fn decode_searxng_base_url(
    read: &mut SettingsRead<'_>,
    below: &Option<String>,
) -> Option<String> {
    let url = read.read(SEARXNG_BASE_URL, below, |url| {
        url.as_deref()
            .map(normalize_search_endpoint)
            .transpose()
            .map(drop)
    });
    url.map(|url| normalize_search_endpoint(&url).unwrap_or(url))
}

/// A search endpoint is never accepted with credentials, a query, or a
/// fragment. Without a usable endpoint the provider cannot stay SearXNG,
/// and falls back to the automatic one.
pub(crate) fn decode_search_provider(
    read: &mut SettingsRead<'_>,
    default: AgentWebSearchProvider,
    has_endpoint: bool,
) -> AgentWebSearchProvider {
    let provider = read
        .fields()
        .read_quiet::<AgentWebSearchProvider>(WEB_SEARCH_PROVIDER, Ok);
    match provider {
        FieldRead::Valid(AgentWebSearchProvider::Searxng) if !has_endpoint => {
            read.fields().invalid(
                WEB_SEARCH_PROVIDER,
                Some("SearXNG has no usable endpoint; Agent web search uses the automatic provider until an endpoint is configured again"),
            );
            read.mark(WEB_SEARCH_PROVIDER);
            AgentWebSearchProvider::Auto
        }
        FieldRead::Valid(provider) => {
            read.mark(WEB_SEARCH_PROVIDER);
            provider
        }
        FieldRead::Missing => default,
        FieldRead::Invalid => {
            read.fields().invalid(
                WEB_SEARCH_PROVIDER,
                Some("the web search provider is not supported; Agent web search uses the automatic provider until one is chosen again"),
            );
            default
        }
    }
}

pub(crate) fn validate_search_pair(settings: &AgentSettings) -> Result<(), String> {
    if settings.web_search_provider == AgentWebSearchProvider::Searxng
        && settings.searxng_base_url.is_none()
    {
        return Err("searxngBaseUrl is required when webSearchProvider is searxng".to_string());
    }
    Ok(())
}

/// The SearXNG endpoint: an endpoint URL (no credentials, query or fragment)
/// within the URL bound, without a trailing `/`. This is the address's one
/// check: web search takes it as valid. The error completes a sentence that
/// starts with the field's name.
pub(crate) fn normalize_search_endpoint(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("cannot be empty".to_string());
    }
    UrlPolicy::ENDPOINT
        .within(MAX_URL_BYTES)
        .parse(raw)
        .map_err(|rejection| rejection.to_string())?;
    Ok(raw.trim_end_matches('/').to_string())
}
