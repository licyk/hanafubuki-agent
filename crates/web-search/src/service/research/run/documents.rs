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

impl Run<'_> {
    pub(super) async fn select_windows(&mut self, round: u8) {
        self.fold_bodies(round).await;
        let live = self
            .bodies
            .iter()
            .filter(|b| b.round == round && b.folded_into.is_none())
            .count();
        let per_doc = 48usize.div_ceil(live.max(1)).max(4);
        for body in self
            .bodies
            .iter_mut()
            .filter(|b| b.round == round && b.folded_into.is_none())
        {
            let mut windows = windows::split(&body.content);
            windows::order(
                &mut windows,
                &windows::tokens(&format!(
                    "{} {}",
                    self.report.question, body.candidate.source.title
                )),
            );
            let slots = 48usize
                .saturating_sub(self.window_used)
                .min(per_doc)
                .min(windows.len());
            let chosen: Vec<_> = windows.iter().take(slots).map(windows::compact).collect();
            self.window_used += slots;
            if let Some(scores) = self
                .service
                .rerank(&self.report.question, &chosen, &self.cancellation)
                .await
            {
                self.evaluated = true;
                let mut scored: Vec<_> = windows.into_iter().take(slots).zip(scores).collect();
                scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.position.cmp(&b.0.position)));
                body.selected = Some(scored.into_iter().take(6).map(|(w, _)| w).collect());
            } else {
                body.selected = Some(windows.into_iter().take(6).collect());
            }
        }
    }
    async fn fold_bodies(&mut self, round: u8) {
        let indices: Vec<_> = self
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| b.round == round)
            .map(|(i, _)| i)
            .collect();
        if indices.len() < 2 {
            return;
        }
        let texts: Vec<_> = indices
            .iter()
            .map(|i| self.bodies[*i].content.chars().take(2000).collect())
            .collect();
        let Some(vectors) = self.service.embeddings(&texts, &self.cancellation).await else {
            return;
        };
        for (left, i) in indices.iter().enumerate() {
            if self.bodies[*i].folded_into.is_some() {
                continue;
            }
            for (right, j) in indices.iter().enumerate().skip(left + 1) {
                let a = &self.bodies[*i];
                let b = &self.bodies[*j];
                if b.folded_into.is_some()
                    || a.candidate.source.published_at.is_none()
                    || a.candidate.source.published_at != b.candidate.source.published_at
                {
                    continue;
                }
                if clusters::cosine(&vectors[left], &vectors[right]) > 0.95
                    && clusters::numerically_consistent(&a.content, &b.content)
                {
                    self.bodies[*j].folded_into =
                        Some(self.bodies[*i].candidate.source.url.clone());
                }
            }
        }
    }
    pub(super) fn render_pending(&mut self) {
        let pending: Vec<_> = self
            .bodies
            .iter()
            .filter(|b| {
                !self
                    .report
                    .documents
                    .iter()
                    .any(|d| d.url == b.candidate.source.url && d.kind == DocumentKind::Body)
            })
            .collect();
        let count = pending.len();
        for (i, body) in pending.into_iter().enumerate() {
            let (content, used, omitted) = if let Some(representative) = &body.folded_into {
                (format!("[body folded into {representative}: cosine > 0.95, same published date, no conflicting numbers; source retained]"),0,0)
            } else {
                let selected = body.selected.clone().unwrap_or_else(|| {
                    let mut windows = windows::split(&body.content);
                    windows::order(&mut windows, &windows::tokens(&self.report.question));
                    windows.into_iter().take(6).collect()
                });
                windows::assemble(selected, self.token_remaining / (count - i).max(1))
            };
            self.token_remaining = self.token_remaining.saturating_sub(used);
            self.report
                .documents
                .retain(|d| d.url != body.candidate.source.url);
            self.report.documents.push(ResearchDocument {
                url: body.candidate.source.url.clone(),
                title: body.candidate.source.title.clone(),
                kind: DocumentKind::Body,
                text_origin: TextOrigin::Original,
                engine_hits: body.candidate.source.engines.clone(),
                published: body.candidate.source.published_at.clone(),
                updated_at: body.updated_at.clone(),
                fetched_at: chrono::DateTime::from_timestamp(body.fetched_at, 0)
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_default(),
                round: body.round,
                cluster: body.candidate.cluster.clone(),
                independence: "unknown".into(),
                tier: if body.folded_into.is_some() {
                    "folded".into()
                } else {
                    body.tier.clone()
                },
                cache: body.cache,
                truncated: body.truncated || omitted > 0,
                content,
            });
        }
    }
}
