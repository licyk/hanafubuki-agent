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

#[test]
fn platform_payloads_keep_good_neighbors_and_native_metadata() {
    let cases = [
        (
            Platform::Github,
            r#"{"items":[false,{"html_url":"https://github.com/o/r","name":"r","full_name":"o/r","description":"async","stargazers_count":9}]}"#,
            "o/r",
            "⭐9",
        ),
        (
            Platform::V2ex,
            r#"[{"id":123,"title":"rust topic","content":"discussion"},{"id":124,"title":"other","content":"unrelated"}]"#,
            "rust topic",
            "discussion",
        ),
        (
            Platform::Bilibili,
            r#"{"code":0,"data":{"result":[{"data":[null,{"arcurl":"https://www.bilibili.com/video/BV1","title":"<em>Rust</em>","desc":"video"}]}]}}"#,
            "Rust",
            "video",
        ),
        (
            Platform::Reddit,
            r#"{"data":{"children":[{"data":{"url":"https://reddit.com/r/rust/1","title":"Rust","selftext":"discussion"}}]}}"#,
            "Rust",
            "discussion",
        ),
        (
            Platform::Hackernews,
            r#"{"hits":[{"objectID":"42","url":null,"story_title":"Rust","points":7,"num_comments":4}]}"#,
            "Rust",
            "7 points",
        ),
        (
            Platform::Stackoverflow,
            r#"{"items":[{"link":"https://stackoverflow.com/questions/42","title":"A &amp; B","score":3,"is_answered":true,"answer_count":2}]}"#,
            "A & B",
            "✓ answered",
        ),
        (
            Platform::Wikipedia,
            r#"{"query":{"search":[{"title":"Rust language","snippet":"<span>systems</span>"}]}}"#,
            "Rust language",
            "systems",
        ),
        (
            Platform::Npm,
            r#"{"objects":[{"package":{"name":"rust","version":"1.2","description":"compiler","links":{}}}]}"#,
            "rust",
            "v1.2",
        ),
        (
            Platform::Juejin,
            r#"{"err_no":0,"data":[{"result_model":{"article_info":{"article_id":"123","title":"Rust","brief_content":"systems"}}}]}"#,
            "Rust",
            "systems",
        ),
        (
            Platform::Qiita,
            r#"[{"url":"https://qiita.com/a/1","title":"Rust","body":"<b>systems</b>"}]"#,
            "Rust",
            "systems",
        ),
        (
            Platform::Douban,
            r#"<div class="result"><h3><a href="https://www.douban.com/link2?url=https%3A%2F%2Fbook.douban.com%2Fsubject%2F1">Rust</a></h3><div class="info">book</div></div>"#,
            "Rust",
            "book",
        ),
    ];
    for (platform, body, title, snippet) in cases {
        let response = parse(platform, "rust", 5, "en", body.as_bytes()).unwrap();
        assert_eq!(response.sources.len(), 1, "{platform:?}");
        assert_eq!(response.sources[0].title, title, "{platform:?}");
        assert!(
            response.sources[0].snippet.contains(snippet),
            "{platform:?}"
        );
        assert!(response.untrusted);
    }
}

#[test]
fn encoded_query_and_language_cannot_change_wikipedia_host() {
    let (base, params) = endpoint(Platform::Wikipedia, "a&b=/", 5, "evil.example/path").unwrap();
    let request = reqwest::Client::new()
        .get(base)
        .query(&params)
        .build()
        .unwrap();
    assert_eq!(request.url().host_str(), Some("en.wikipedia.org"));
    assert_eq!(
        request
            .url()
            .query_pairs()
            .find(|(key, _)| key == "srsearch")
            .unwrap()
            .1,
        "a&b=/"
    );
}

#[test]
fn provider_level_error_is_not_an_empty_success() {
    for (platform, body) in [
        (Platform::Bilibili, r#"{"code":-412,"data":{"result":[]}}"#),
        (
            Platform::Stackoverflow,
            r#"{"error_message":"quota","items":[]}"#,
        ),
        (Platform::Juejin, r#"{"err_no":1,"data":[]}"#),
    ] {
        assert!(parse(platform, "rust", 5, "en", body.as_bytes()).is_err());
    }
}
