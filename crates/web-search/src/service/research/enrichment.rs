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

//! Optional ranking and decision services; unavailable services leave rule results intact.
use super::{
    config::configured_or_environment,
    intent::{Intent, Rewrite},
};
use crate::WebSearchService;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

impl WebSearchService {
    pub(in crate::service::research) async fn rerank(
        &self,
        query: &str,
        documents: &[String],
        cancel: &CancellationToken,
    ) -> Option<Vec<f64>> {
        if documents.is_empty() {
            return None;
        }
        let options = &self.inner.config.search.research;
        let key = configured_or_environment(&options.rerank_key, "SILICONFLOW_RERANK_API_KEY");
        if key.is_empty() {
            return None;
        }
        let body = RerankRequest {
            model: "BAAI/bge-reranker-v2-m3",
            query,
            documents,
            return_documents: false,
        };
        let bytes = self
            .send_bounded(
                self.inner
                    .client
                    .post(format!(
                        "{}/rerank",
                        options.rerank_endpoint.trim_end_matches('/')
                    ))
                    .bearer_auth(key)
                    .timeout(Duration::from_secs(15))
                    .json(&body),
                cancel,
            )
            .await
            .ok()?;
        let response: RerankResponse = serde_json::from_slice(&bytes).ok()?;
        let mut scores = vec![0.0; documents.len()];
        let mut valid = false;
        for entry in response.results {
            if let Some(score) = scores.get_mut(entry.index) {
                if entry.relevance_score.is_finite() {
                    *score = entry.relevance_score;
                    valid = true;
                }
            }
        }
        valid.then_some(scores)
    }
    pub(in crate::service::research) async fn embeddings(
        &self,
        texts: &[String],
        cancel: &CancellationToken,
    ) -> Option<Vec<Vec<f64>>> {
        if texts.len() < 2 {
            return None;
        }
        let options = &self.inner.config.search.research;
        let key = configured_or_environment(&options.rerank_key, "SILICONFLOW_RERANK_API_KEY");
        if key.is_empty() {
            return None;
        }
        let body = EmbeddingRequest {
            model: "BAAI/bge-m3",
            input: texts,
        };
        let bytes = self
            .send_bounded(
                self.inner
                    .client
                    .post(format!(
                        "{}/embeddings",
                        options.rerank_endpoint.trim_end_matches('/')
                    ))
                    .bearer_auth(key)
                    .timeout(Duration::from_secs(15))
                    .json(&body),
                cancel,
            )
            .await
            .ok()?;
        let response: EmbeddingResponse = serde_json::from_slice(&bytes).ok()?;
        let mut vectors = vec![None; texts.len()];
        for entry in response.data {
            if let Some(slot) = vectors.get_mut(entry.index) {
                *slot = Some(entry.embedding);
            }
        }
        let vectors: Vec<Vec<f64>> = vectors.into_iter().collect::<Option<_>>()?;
        let width = vectors.first()?.len();
        (width > 0
            && vectors
                .iter()
                .all(|v| v.len() == width && v.iter().all(|x| x.is_finite())))
        .then_some(vectors)
    }
    pub(in crate::service::research) async fn decide_intent(
        &self,
        question: &str,
        intent: &mut Intent,
        cancel: &CancellationToken,
    ) {
        let questions = BTreeMap::from([
            (
                "qtype".into(),
                DecisionQuestion {
                    kind: "choice",
                    instructions: "查询类型分类".into(),
                    criteria: Criteria::Choices(BTreeMap::from([
                        ("fact".into(), "事实快查，答案唯一".into()),
                        ("research".into(), "需要多源综合的深度研究".into()),
                        ("timely".into(), "追踪最新动态/新闻".into()),
                        ("nav".into(), "导航到具体站点或页面".into()),
                    ])),
                },
            ),
            (
                "breadth".into(),
                DecisionQuestion {
                    kind: "score",
                    instructions: "需要多大搜索广度？".into(),
                    criteria: Criteria::Scale(vec![
                        "单引擎即可".into(),
                        "2-3 引擎并发".into(),
                        "全源交叉验证".into(),
                    ]),
                },
            ),
        ]);
        let Some(answers) = self
            .decide(&format!("搜索查询：{question}"), questions, cancel)
            .await
        else {
            return;
        };
        if let Some(choice) = answers.get("qtype").and_then(|a| a.choice.as_deref()) {
            if ["fact", "research", "timely", "nav"].contains(&choice) {
                intent.task = if choice == "timely" {
                    "research"
                } else {
                    choice
                }
                .into();
                intent.source = "decision".into();
            }
        }
        if let Some(score) = answers
            .get("breadth")
            .and_then(|a| a.score)
            .filter(|s| s.is_finite())
        {
            intent.breadth = score.clamp(0.0, 3.0);
        }
    }
    pub(in crate::service::research) async fn sufficient(
        &self,
        question: &str,
        brief: &str,
        cancel: &CancellationToken,
    ) -> Option<f64> {
        let questions = BTreeMap::from([(
            "sufficient".into(),
            DecisionQuestion {
                kind: "noul",
                instructions: "当前证据是否足以回答该问题".into(),
                criteria: Criteria::Choices(BTreeMap::from([
                    ("true".into(), "选中正文可直接支撑该问题的答案".into()),
                    ("false".into(), "证据缺失、只覆盖单方或不可靠".into()),
                ])),
            },
        )]);
        self.decide(
            &format!("问题：{question}\n证据概要：{brief}"),
            questions,
            cancel,
        )
        .await?
        .get("sufficient")?
        .probability()
    }
    pub(in crate::service::research) async fn pick_rewrite(
        &self,
        question: &str,
        missing: &[String],
        rewrites: &[Rewrite],
        cancel: &CancellationToken,
    ) -> usize {
        if rewrites.len() < 2 {
            return 0;
        }
        let criteria = rewrites
            .iter()
            .enumerate()
            .map(|(i, r)| (format!("c{}", i + 1), r.declares.clone()))
            .collect();
        let questions = BTreeMap::from([(
            "pick".into(),
            DecisionQuestion {
                kind: "choice",
                instructions: "选出最可能补齐证据缺点的查询改写".into(),
                criteria: Criteria::Choices(criteria),
            },
        )]);
        self.decide(
            &format!("问题：{question}\n证据缺口：{}", missing.join("; ")),
            questions,
            cancel,
        )
        .await
        .and_then(|a| a.get("pick").and_then(|a| a.choice.clone()))
        .and_then(|s| s.strip_prefix('c').and_then(|n| n.parse::<usize>().ok()))
        .filter(|n| *n > 0 && *n <= rewrites.len())
        .map(|n| n - 1)
        .unwrap_or(0)
    }
    async fn decide(
        &self,
        state: &str,
        questions: BTreeMap<String, DecisionQuestion>,
        cancel: &CancellationToken,
    ) -> Option<BTreeMap<String, DecisionAnswer>> {
        let options = &self.inner.config.search.research;
        let endpoint = configured_or_environment(&options.decision_endpoint, "DECISION_MODEL_URL");
        let key = configured_or_environment(&options.decision_key, "DECISION_MODEL_API_KEY");
        if endpoint.is_empty() || key.is_empty() {
            return None;
        }
        let body = DecisionRequest {
            model: "decision-model-preview",
            state,
            questions,
        };
        let bytes = self
            .send_bounded(
                self.inner
                    .client
                    .post(endpoint)
                    .bearer_auth(key)
                    .timeout(Duration::from_secs(12))
                    .json(&body),
                cancel,
            )
            .await
            .ok()?;
        serde_json::from_slice::<DecisionResponse>(&bytes)
            .ok()
            .map(|r| r.answers)
    }
}
#[derive(Serialize)]
struct RerankRequest<'a> {
    model: &'static str,
    query: &'a str,
    documents: &'a [String],
    return_documents: bool,
}
#[derive(Deserialize)]
struct RerankResponse {
    #[serde(default)]
    results: Vec<Score>,
}
#[derive(Deserialize)]
struct Score {
    index: usize,
    #[serde(default)]
    relevance_score: f64,
}
#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'static str,
    input: &'a [String],
}
#[derive(Deserialize)]
struct EmbeddingResponse {
    #[serde(default)]
    data: Vec<Embedding>,
}
#[derive(Deserialize)]
struct Embedding {
    index: usize,
    embedding: Vec<f64>,
}
#[derive(Serialize)]
struct DecisionRequest<'a> {
    model: &'static str,
    state: &'a str,
    questions: BTreeMap<String, DecisionQuestion>,
}
#[derive(Serialize)]
struct DecisionQuestion {
    #[serde(rename = "type")]
    kind: &'static str,
    instructions: String,
    criteria: Criteria,
}
#[derive(Serialize)]
#[serde(untagged)]
enum Criteria {
    Choices(BTreeMap<String, String>),
    Scale(Vec<String>),
}
#[derive(Deserialize)]
struct DecisionResponse {
    #[serde(default)]
    answers: BTreeMap<String, DecisionAnswer>,
}
#[derive(Deserialize)]
struct DecisionAnswer {
    choice: Option<String>,
    value: Option<String>,
    score: Option<f64>,
    confidence: Option<f64>,
    noul: Option<Probability>,
    p_yes: Option<Probability>,
    #[serde(rename = "pYes")]
    p_yes_camel: Option<Probability>,
    probability: Option<Probability>,
    yes: Option<f64>,
    #[serde(rename = "true")]
    truth: Option<f64>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Probability {
    Number(f64),
    Named {
        yes: Option<f64>,
        #[serde(rename = "true")]
        truth: Option<f64>,
    },
}
impl DecisionAnswer {
    fn probability(&self) -> Option<f64> {
        let probability = self
            .noul
            .as_ref()
            .or(self.p_yes.as_ref())
            .or(self.p_yes_camel.as_ref())
            .or(self.probability.as_ref());
        let value = probability
            .and_then(|p| match p {
                Probability::Number(n) => Some(*n),
                Probability::Named { yes, truth } => yes.or(*truth),
            })
            .or(self.yes)
            .or(self.truth)
            .or_else(|| {
                let choice = self.choice.as_deref().or(self.value.as_deref())?;
                let confidence = self.confidence.filter(|n| (0.0..=1.0).contains(n))?;
                match choice {
                    "yes" | "true" => Some(confidence),
                    "no" | "false" => Some(1.0 - confidence),
                    _ => None,
                }
            })?;
        (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
    }
}
