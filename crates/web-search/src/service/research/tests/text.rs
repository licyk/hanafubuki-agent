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

use super::super::{clusters, intent, windows};

#[test]
fn windows_keep_late_relevant_text_and_atomic_structures() {
    let text=format!("{}\n\nlate decisive rust cancellation evidence.\n\n| header |\n| --- |\n| value |\n\n```rust\nfn main() {{}}\n```", "unrelated sentence. ".repeat(100));
    let mut split = windows::split(&text);
    assert!(split
        .iter()
        .filter(|w| w.kind == windows::WindowKind::Paragraph)
        .all(|w| w.text.chars().count() <= 512));
    assert_eq!(
        split
            .iter()
            .filter(|w| w.kind == windows::WindowKind::Code)
            .count(),
        1
    );
    assert_eq!(
        split
            .iter()
            .filter(|w| w.kind == windows::WindowKind::Table)
            .count(),
        1
    );
    windows::order(&mut split, &windows::tokens("rust cancellation evidence"));
    assert!(split[0].text.contains("decisive"));
    let (assembled, _, _) = windows::assemble(split.into_iter().take(2).collect(), 600);
    assert!(assembled.contains("decisive"));
    assert!(assembled.contains('¶'));
}

#[test]
fn gap_rewrites_keep_versions_dates_negation_and_error_codes() {
    let question = "比较 Qwen3.8 和 GLM 5.1 本地部署，不支持 HTTP_500，2026-11-14";
    let sides = intent::comparison_sides(question).unwrap();
    assert_eq!(sides[0], "Qwen3.8");
    assert_eq!(sides[1], "GLM 5.1");
    let rewrites = intent::rewrites(question, &["GLM 5.1".into()], true);
    assert!(!rewrites.is_empty());
    for rewrite in rewrites {
        for protected in ["3.8", "5.1", "不", "HTTP_500", "2026-11-14"] {
            assert!(rewrite.query.contains(protected));
        }
    }
    assert!(!clusters::numerically_consistent(
        "value 12.5",
        "value 99.0"
    ));
    assert!(clusters::numerically_consistent(
        "value 12.5",
        "12.5 is the value"
    ));
}
