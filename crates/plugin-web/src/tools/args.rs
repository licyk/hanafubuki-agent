//! Model arguments of the web tools. A missing or `null` optional field
//! means "use the default" (strict function schemas list every property as
//! required-and-nullable, but omitting one means the same), and an unknown
//! field is ignored. The limit fields (`maximumResults`,
//! `maximumContentBytes`, `fromOffset`) are read from the argument object
//! by the tools themselves, clamped into their bounds with a note.

use hanafubuki_agent_web_search::{WebSearchCategory, WebSearchProvider, WebSearchTimeRange};
use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WebSearchArguments {
    pub query: String,
    #[serde(default, deserialize_with = "nullable_default")]
    pub provider: WebSearchProvider,
    #[serde(default, deserialize_with = "nullable_default")]
    pub category: WebSearchCategory,
    #[serde(default)]
    pub time_range: Option<WebSearchTimeRange>,
    #[serde(default)]
    pub region: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FetchWebPageArguments {
    pub url: String,
    #[serde(default)]
    pub content_generation: Option<String>,
}

pub(super) const DEFAULT_SEARCH_RESULTS: usize = 5;
pub(super) const DEFAULT_FETCH_CONTENT_BYTES: usize = 50_000;

fn nullable_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}
