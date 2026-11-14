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

use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FetchPreference {
    #[default]
    Exa,
    Firecrawl,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FetchOptions {
    pub allow_fake_ip_cidrs: Vec<String>,
    pub prefer: FetchPreference,
    pub browser_enabled: bool,
    pub exa_endpoint: String,
    pub firecrawl_endpoint: String,
    pub crawl4ai_endpoint: String,
    pub crawl4ai_api_key: String,
}
impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            allow_fake_ip_cidrs: Vec::new(),
            prefer: FetchPreference::Exa,
            browser_enabled: true,
            exa_endpoint: "https://api.exa.ai/contents".into(),
            firecrawl_endpoint: "https://api.firecrawl.dev/v2/scrape".into(),
            crawl4ai_endpoint: "https://api.crawl4ai.com/scrape".into(),
            crawl4ai_api_key: String::new(),
        }
    }
}
impl std::fmt::Debug for FetchOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchOptions")
            .field("prefer", &self.prefer)
            .field("browser_enabled", &self.browser_enabled)
            .finish_non_exhaustive()
    }
}
