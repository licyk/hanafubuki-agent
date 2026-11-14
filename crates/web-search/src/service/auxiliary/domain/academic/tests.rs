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
fn atom_preserves_urls_and_published_dates() {
    let sources = atom(
        r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><entry><id>https://arxiv.org/abs/1234.5678</id><title> A &amp; B </title><summary>Some evidence</summary><published>2026-11-14T00:00:00Z</published></entry></feed>"#,
        "arxiv",
    );
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].url, "https://arxiv.org/abs/1234.5678");
    assert_eq!(sources[0].title, "A & B");
    assert_eq!(sources[0].published_at.as_deref(), Some("2026-11-14"));
    let sources = atom(
        r#"<feed><entry><title>Act</title><link href="http://www.legislation.gov.uk/id/1"/><link href="http://www.legislation.gov.uk/1"/><updated>2026-11-14</updated></entry></feed>"#,
        "uk_legislation",
    );
    assert_eq!(sources[0].url, "https://www.legislation.gov.uk/1");
}
