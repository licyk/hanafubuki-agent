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
use serde_json::json;
#[test]
fn settings_bind_engine_keys_research_and_fetch_without_exposing_secrets() {
    let options=search_options_from_value(json!({"provider":"multi","exaApiKeys":"one,two","exaEndpoint":"https://gateway.example/search","researchPrimaryPool":["exa","unknown"],"rerankKey":"secret-rank","fetchPrefer":"firecrawl","browserFetch":false,"allowFakeIpCidrs":["198.18.0.0/15"],"platforms":["github"],"cacheTtlMinutes":2})).unwrap();
    assert_eq!(options.provider, E::Multi);
    assert_eq!(options.research.primary_pool, vec![E::Exa]);
    assert_eq!(options.research.rerank_key, "secret-rank");
    assert!(!options.fetch.browser_enabled);
    assert_eq!(options.cache_ttl_minutes, 2);
    assert!(!format!("{options:?}").contains("secret-rank"));
    let schema = web_option_schema();
    let keys: std::collections::BTreeSet<_> = schema.iter().map(|s| s.key.clone()).collect();
    assert_eq!(keys.len(), schema.len());
}
#[test]
fn model_endpoints_cannot_embed_credentials() {
    assert!(
        search_options_from_value(json!({"exaEndpoint":"https://user:secret@example.com/"}))
            .is_err()
    );
    assert!(search_options_from_value(
        json!({"rerankEndpoint":"https://user:secret@example.com/"})
    )
    .is_err());
}
