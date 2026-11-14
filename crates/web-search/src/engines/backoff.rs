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

//! Error bodies are inspected only to classify retry policy; never retained.
use super::FailureClass;
use std::time::Duration;

pub(crate) fn delay(text: &str) -> Option<Duration> {
    let text = text.to_ascii_lowercase();
    for pattern in [
        r"retry[- ]?after[:\s]+(\d{1,7})\s*(milliseconds?|ms|seconds?|secs?|s|minutes?|mins?|m|hours?|h)?",
        r"resets?\s+in\s+(\d{1,4})\s*(seconds?|secs?|minutes?|mins?|hours?|hrs?|days?)",
    ] {
        let regex = regex::Regex::new(pattern).ok()?;
        if let Some(c) = regex.captures(&text) {
            let amount = c.get(1)?.as_str().parse::<u64>().ok()?;
            let unit = c.get(2).map(|v| v.as_str()).unwrap_or("s");
            let scale = if unit == "ms" || unit.starts_with("millisecond") {
                1
            } else if unit.starts_with('h') {
                3_600_000
            } else if unit.starts_with('d') {
                86_400_000
            } else if unit.starts_with('m') {
                60_000
            } else {
                1000
            };
            return Some(Duration::from_millis(
                amount.saturating_mul(scale).clamp(5000, 7 * 86_400_000),
            ));
        }
    }
    let regex = regex::Regex::new(
        r"resets?\s+(?:at|on)\s+(\d{4}-\d{2}-\d{2}[t ]\d{2}:\d{2}(?::\d{2})?(?:\.\d+)?z?)",
    )
    .ok()?;
    if let Some(c) = regex.captures(&text) {
        let mut timestamp = c[1].replace([' ', 't'], "T").replace('z', "Z");
        if !timestamp.ends_with('Z') {
            timestamp.push('Z');
        }
        if let Ok(date) = chrono::DateTime::parse_from_rfc3339(&timestamp) {
            return Some(Duration::from_secs(
                (date.timestamp() - chrono::Utc::now().timestamp()).clamp(5, 7 * 86400) as u64,
            ));
        }
    }
    ["monthly", "per month", "this month"]
        .iter()
        .any(|s| text.contains(s))
        .then_some(Duration::from_secs(86400))
}
pub(crate) fn classify(status: u16, text: &str) -> (FailureClass, Option<Duration>) {
    let lower = text.to_ascii_lowercase();
    if [
        "eligibility check failed",
        "available in your location",
        "available in your region",
        "available in your country",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        return (FailureClass::Region, Some(Duration::from_secs(86400)));
    }
    let class = if status == 402
        || [
            "quota",
            "insufficient credit",
            "credits exhausted",
            "limit exceeded",
            "balance exhausted",
        ]
        .iter()
        .any(|s| lower.contains(s))
    {
        FailureClass::Quota
    } else if [
        "captcha",
        "cloudflare",
        "verify you are human",
        "bot detection",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        FailureClass::BotWall
    } else if matches!(status, 401 | 403) {
        FailureClass::Auth
    } else if status == 429 || status >= 500 {
        FailureClass::Transient
    } else {
        FailureClass::Unknown
    };
    (class, delay(&lower))
}
