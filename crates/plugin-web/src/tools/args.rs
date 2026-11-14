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
    #[serde(default)]
    pub engines: Option<Vec<WebSearchProvider>>,
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
