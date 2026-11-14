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
fn chinese_pages_decode_declared_encodings_without_losing_metadata() {
    let html = "<html><head><meta charset='gbk'><meta property='article:modified_time' content='2026-11-14'></head><body>中文证据</body></html>";
    let (bytes, _, errors) = encoding_rs::GBK.encode(html);
    assert!(!errors);
    for content_type in ["text/html", "text/html; charset=gbk"] {
        let decoded = decode(&bytes, content_type);
        assert!(String::from_utf8_lossy(&decoded).contains("中文证据"));
        assert_eq!(
            super::super::metadata::updated(&decoded).as_deref(),
            Some("2026-11-14")
        );
    }
    assert!(super::super::metadata::updated(
        b"<meta property='article:modified_time' content='not a date'>"
    )
    .is_none());
}
