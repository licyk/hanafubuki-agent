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

//! Sentence windows, preserving code fences and tables as indivisible blocks.
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Window {
    pub text: String,
    pub position: usize,
    pub kind: WindowKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WindowKind {
    Paragraph,
    Code,
    Table,
}

pub(super) fn estimate_tokens(text: &str) -> usize {
    let total = text.chars().count();
    let cjk=text.chars().filter(|c|matches!(*c,'\u{4e00}'..='\u{9fff}'|'\u{3040}'..='\u{30ff}'|'\u{ac00}'..='\u{d7af}')).count();
    (cjk as f64 / 1.5 + (total - cjk) as f64 / 4.0).ceil() as usize
}
pub(super) fn split(text: &str) -> Vec<Window> {
    let mut windows = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        text_windows(&rest[..start], &mut windows);
        let end = rest[start + 3..]
            .find("```")
            .map(|n| start + 3 + n + 3)
            .unwrap_or(rest.len());
        push(&mut windows, &rest[start..end], WindowKind::Code);
        rest = &rest[end..];
    }
    text_windows(rest, &mut windows);
    windows
}
fn text_windows(text: &str, windows: &mut Vec<Window>) {
    let lines: Vec<_> = text.lines().collect();
    let mut paragraph = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim_start().starts_with('|') {
            let start = index;
            while index < lines.len() && lines[index].trim_start().starts_with('|') {
                index += 1;
            }
            if index - start >= 2 {
                paragraph_windows(&paragraph.join(" "), windows);
                paragraph.clear();
                push(windows, &lines[start..index].join("\n"), WindowKind::Table);
                continue;
            }
            paragraph.push(lines[start].trim());
            continue;
        }
        if lines[index].trim().is_empty() {
            paragraph_windows(&paragraph.join(" "), windows);
            paragraph.clear();
        } else {
            paragraph.push(lines[index].trim());
        }
        index += 1;
    }
    paragraph_windows(&paragraph.join(" "), windows);
}
fn paragraph_windows(text: &str, windows: &mut Vec<Window>) {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut current = String::new();
    for sentence in normalized.split_inclusive(['。', '！', '？', '!', '?', '.']) {
        let mut chars = sentence.chars().peekable();
        while chars.peek().is_some() {
            let piece: String = chars.by_ref().take(512).collect();
            if current.chars().count() + piece.chars().count() > 512 {
                push(windows, &current, WindowKind::Paragraph);
                current.clear();
            }
            current.push_str(&piece);
            if chars.peek().is_some() {
                push(windows, &current, WindowKind::Paragraph);
                current.clear();
            }
        }
    }
    push(windows, &current, WindowKind::Paragraph);
}
fn push(windows: &mut Vec<Window>, text: &str, kind: WindowKind) {
    if !text.trim().is_empty() {
        windows.push(Window {
            text: text.trim().into(),
            position: windows.len(),
            kind,
        });
    }
}
pub(super) fn tokens(query: &str) -> BTreeSet<String> {
    let lower = query.to_lowercase();
    let stops = [
        "what",
        "which",
        "how",
        "why",
        "when",
        "where",
        "who",
        "is",
        "are",
        "was",
        "the",
        "and",
        "for",
        "with",
        "vs",
        "compare",
        "difference",
        "between",
        "better",
        "best",
        "还是",
        "什么",
        "怎么",
        "哪个",
        "哪些",
        "如何",
        "区别",
        "差异",
        "对比",
        "比较",
        "优劣",
    ];
    let mut tokens: BTreeSet<_> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || "+#.-".contains(c)))
        .filter(|s| {
            s.len() >= 3
                && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && !stops.contains(s)
        })
        .map(String::from)
        .collect();
    for run in lower
        .split(|c: char| !matches!(c, '\u{4e00}'..='\u{9fff}'))
        .filter(|s| !s.is_empty())
    {
        let chars: Vec<_> = run.chars().collect();
        if chars.len() <= 2 {
            tokens.insert(run.into());
        } else {
            for pair in chars.windows(2) {
                tokens.insert(pair.iter().collect());
            }
        }
    }
    tokens
}
pub(super) fn order(windows: &mut [Window], tokens: &BTreeSet<String>) {
    windows.sort_by_key(|w| {
        let lower = w.text.to_lowercase();
        (
            std::cmp::Reverse(
                tokens
                    .iter()
                    .filter(|token| lower.contains(token.as_str()))
                    .count(),
            ),
            w.position,
        )
    });
}
pub(super) fn compact(window: &Window) -> String {
    window
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(400)
        .collect()
}
pub(super) fn assemble(mut windows: Vec<Window>, budget: usize) -> (String, usize, usize) {
    windows.sort_by_key(|w| w.position);
    let mut lines = Vec::new();
    let mut used = 0;
    let mut omitted = 0;
    let mut kinds = BTreeSet::new();
    for w in windows {
        let piece = format!("¶{} {}", w.position + 1, w.text);
        let estimate = estimate_tokens(&piece);
        if used + estimate > budget {
            omitted += 1;
            if w.kind != WindowKind::Paragraph {
                kinds.insert(w.kind);
            }
            continue;
        }
        lines.push(piece);
        used += estimate;
    }
    if omitted > 0 {
        lines.push(format!(
            "[{omitted} window(s) omitted: token budget{}]",
            if kinds.is_empty() {
                String::new()
            } else {
                format!(
                    " ({})",
                    kinds
                        .into_iter()
                        .map(|k| if k == WindowKind::Code {
                            "code"
                        } else {
                            "table"
                        })
                        .collect::<Vec<_>>()
                        .join("/")
                )
            }
        ));
    }
    (lines.join("\n\n"), used, omitted)
}
