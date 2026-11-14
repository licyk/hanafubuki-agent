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
fn youtube_walks_unknown_wrappers_and_skips_malformed_video() {
    let html = r#"<script>var ytInitialData = {"newWrapper":{"items":[null,false,2,"{ text }",{"videoRenderer":{"videoId":7}},{"videoRenderer":{"videoId":"abc","title":{"runs":[{"text":"Rust"}]},"ownerText":{"runs":[{"text":"Author"}]},"lengthText":{"simpleText":"3:00"}}}]}}; after();</script>"#;
    let sources = parse_youtube(html).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].url, "https://www.youtube.com/watch?v=abc");
    assert_eq!(sources[0].title, "Rust");
    assert!(sources[0].snippet.contains("3:00"));
}

#[test]
fn bing_decodes_html_attribute_and_ddg_uses_content_as_url() {
    let bing = parse_bing(
        r#"<div vrhm="{&quot;murl&quot;:&quot;https://example.com/video&quot;,&quot;vt&quot;:&quot;A &amp; B&quot;,&quot;du&quot;:&quot;1:00&quot;}"></div><div vrhm="invalid"></div>"#,
    );
    assert_eq!(bing.len(), 1);
    assert_eq!(bing[0].title, "A & B");
    let ddg = parse_ddg(r#"callback({"results":[false,{"content":"https://example.com/new","url":"https://example.com/old","title":"Video","duration":"1:00"}]});"#).unwrap();
    assert_eq!(ddg.len(), 1);
    assert_eq!(ddg[0].url, "https://example.com/new");
    assert!(parse_ddg("nothing").is_err());
}
