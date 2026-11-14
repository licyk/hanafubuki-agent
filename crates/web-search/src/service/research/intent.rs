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

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Intent {
    pub task: String,
    pub comparison: bool,
    pub freshness: Freshness,
    pub language: String,
    pub breadth: f64,
    pub source: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Freshness {
    None,
    Day,
    Week,
    Month,
}
impl Freshness {
    pub fn cache_age(self) -> i64 {
        match self {
            Self::Day => 86400,
            Self::Week | Self::None => 7 * 86400,
            Self::Month => 7 * 86400,
        }
    }
}
fn matches(pattern: &str, text: &str) -> bool {
    Regex::new(pattern).is_ok_and(|r| r.is_match(text))
}
pub(super) fn judge(question: &str) -> Intent {
    let comparison = matches(
        r"(?i)(对比|比较|区别|差异|哪个好|哪个更好|哪个更|优劣势|优缺点|还是好|vs\.?\s|\bversus\b)",
        question,
    );
    let task = if !comparison
        && matches(
            "官网|官方网站|下载地址|下载页|镜像站|仓库地址|项目主页",
            question,
        ) {
        "nav"
    } else if !comparison && matches("是什么|什么是|含义|定义|是指|意思|指的是", question)
    {
        "fact"
    } else {
        "research"
    };
    let freshness = if matches(r"(?i)今天|今日|刚刚|几个小时|24\s?小时|today", question)
    {
        Freshness::Day
    } else if matches(
        r"(?i)本周|这周|最近一周|近一周|过去\s?7天|近7天|7天内|last week|this week",
        question,
    ) {
        Freshness::Week
    } else if matches(
        r"(?i)最新|最近|近期|本月|这个月|近一个月|近30天|过去一个月|latest|recent|last month|this month",
        question,
    ) {
        Freshness::Month
    } else {
        Freshness::None
    };
    let cjk = question
        .chars()
        .filter(|c| matches!(*c, '\u{4e00}'..='\u{9fff}'))
        .count();
    Intent {
        task: task.into(),
        comparison,
        freshness,
        language: if cjk as f64 > question.chars().count() as f64 * 0.3 {
            "zh"
        } else {
            "en"
        }
        .into(),
        breadth: 1.0,
        source: "rules".into(),
    }
}
pub(super) fn comparison_sides(question: &str) -> Option<Vec<String>> {
    let split = Regex::new(r"(?i)\s*(?:和|与|跟|同|还是|versus|vs\.?)\s*").ok()?;
    let prefix =
        Regex::new(r"^(请问|帮我|我想知道|比较一下|对比一下|分析一下|说说|讲讲|对比|比较)").ok()?;
    let tail=Regex::new(r"(的)?(优缺点|优劣势|区别|差异|对比|比较|性能|价格|速度|哪个好|哪个更好|哪个更适合|怎么样|如何|特点|利弊).*").ok()?;
    let latin =
        Regex::new(r"^\s*([A-Za-z0-9_.\-+#]+(?:\s+[A-Za-z0-9_.\-+#]+)*)\s*[\x{4e00}-\x{9fff}].*$")
            .ok()?;
    let sides: Vec<_> = split
        .split(question)
        .map(|s| {
            let s = prefix.replace(s.trim(), "");
            let s = tail.replace(&s, "");
            latin
                .replace(&s, "$1")
                .trim_end_matches(|c: char| c.is_whitespace() || "，。？！,.?!".contains(c))
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .take(2)
        .collect();
    (sides.len() == 2 && sides.iter().all(|s| s.chars().count() <= 40)).then_some(sides)
}
pub(super) fn missing_sides(
    documents: &[super::ResearchDocument],
    sides: &[String],
) -> Vec<String> {
    let corpus = documents
        .iter()
        .map(|d| format!("{} {}", d.title, d.content))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    sides
        .iter()
        .filter(|s| !corpus.contains(&s.to_lowercase()))
        .cloned()
        .collect()
}
#[derive(Clone, Debug)]
pub(super) struct Rewrite {
    pub query: String,
    pub declares: String,
}
pub(super) fn rewrites(question: &str, missing: &[String], comparison: bool) -> Vec<Rewrite> {
    let mut proposals: Vec<_> = missing
        .iter()
        .map(|side| Rewrite {
            query: format!("{side} 特性 官方文档 {question}"),
            declares: format!("补缺方证据：{side}"),
        })
        .collect();
    proposals.push(Rewrite {
        query: format!("{question} 官方文档 changelog release notes"),
        declares: "来源限定：官方文档/changelog".into(),
    });
    if let Ok(pattern) = Regex::new("请问|帮我|我想知道|分析一下|说说|讲讲|一下")
    {
        proposals.push(Rewrite {
            query: format!(
                "{}{}",
                pattern.replace_all(question, ""),
                if comparison { " 对比 评测" } else { "" }
            ),
            declares: "查询精化：去疑问套话/补比较词".into(),
        });
    }
    let protected: Vec<String> = [
        r"(?i)\bv?\d+\.\d+(\.\d+)?\w*\b",
        r"\d{4}[-/]\d{1,2}([-/]\d{1,2})?|\d{4}年\d{1,2}月|\b(19|20)\d{2}\b",
        r"(?i)不|没|无法|不能|禁止|无需|cannot|can'?t|not\b|don'?t|won'?t|fail(s|ed)? to",
        r"\b[A-Z][A-Z0-9]*(?:[-_][A-Z0-9]+)+\b|\bE[A-Z]{2,}\b|\bHTTP \d{3}\b|\b0x[0-9A-Fa-f]+\b",
    ]
    .into_iter()
    .filter_map(|p| Regex::new(p).ok())
    .flat_map(|p| {
        p.find_iter(question)
            .map(|m| m.as_str().to_lowercase())
            .collect::<Vec<_>>()
    })
    .collect();
    let normalize = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let mut seen = std::collections::BTreeSet::from([normalize(question)]);
    proposals
        .into_iter()
        .filter(|r| {
            protected.iter().all(|p| r.query.to_lowercase().contains(p))
                && seen.insert(normalize(&r.query))
        })
        .take(3)
        .collect()
}
