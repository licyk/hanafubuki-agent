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

//! Publication/update dates remain source metadata, never evidence of freshness.
pub(super) fn updated(body: &[u8]) -> Option<String> {
    let html = String::from_utf8_lossy(body);
    let document = dom_query::Document::from(html.as_ref());
    for selector in [
        "meta[property='article:modified_time']",
        "meta[property='og:updated_time']",
        "meta[name='last-modified']",
        "time[datetime]",
    ] {
        for node in document.select(selector).iter() {
            if let Some(value) = node.attr(if selector.starts_with("time") {
                "datetime"
            } else {
                "content"
            }) {
                if valid(&value) {
                    return Some(value.to_string());
                }
            }
        }
    }
    let pattern = regex::Regex::new(r#"(?i)"dateModified"\s*:\s*"([^"]+)""#).ok()?;
    let found = pattern
        .captures_iter(&html)
        .filter_map(|c| c.get(1).map(|v| v.as_str()))
        .find(|s| valid(s))
        .map(String::from);
    found
}
fn valid(value: &str) -> bool {
    value.len() <= 128
        && (chrono::DateTime::parse_from_rfc3339(value).is_ok()
            || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok())
}
