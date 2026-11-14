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
    pub(super) async fn followup(&mut self) {
        if !self.intent.comparison {
            return;
        }
        let Some(sides) = intent::comparison_sides(&self.report.question) else {
            self.report
                .notes
                .push("coverage: comparison sides undetermined; unknown".into());
            return;
        };
        let missing = intent::missing_sides(&self.report.documents, &sides);
        for side in &missing {
            self.report.notes.push(format!(
                "evidence gap: side {side:?} not covered (heuristic check)"
            ));
        }
        if missing.is_empty() {
            return;
        }
        if !self.evaluated || self.bodies.is_empty() {
            self.report.notes.push(
                "gate: notEvaluated; rewrite requires successful rerank and fetched evidence"
                    .into(),
            );
            return;
        }
        if self.budget.used() >= 12 {
            self.report
                .notes
                .push("gate: search budget exhausted; rewrite skipped".into());
            return;
        }
        let brief = self
            .report
            .documents
            .iter()
            .filter(|d| d.kind == DocumentKind::Body)
            .take(3)
            .map(|d| {
                format!(
                    "[{}] {}",
                    d.title,
                    d.content.chars().take(200).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let Some(probability) = self
            .service
            .sufficient(&self.report.question, &brief, &self.cancellation)
            .await
        else {
            self.report
                .notes
                .push("gate: decision unavailable or undecidable; rewrite skipped".into());
            return;
        };
        if probability >= 0.6 {
            self.report.notes.push(format!(
                "gate: P(yes)={probability:.2} >= 0.6; rewrite skipped"
            ));
            return;
        }
        let rewrites = intent::rewrites(&self.report.question, &missing, true);
        let picked = self
            .service
            .pick_rewrite(
                &self.report.question,
                &missing,
                &rewrites,
                &self.cancellation,
            )
            .await;
        let Some(rewrite) = rewrites.get(picked) else {
            return;
        };
        self.report
            .notes
            .push(format!("rewrite: {} — {}", rewrite.query, rewrite.declares));
        let pool = self
            .service
            .inner
            .config
            .search
            .research
            .escalation_pool
            .clone();
        let before = self.bodies.len();
        match self.search_round(&rewrite.query, pool, 2).await {
            Ok(candidates) => {
                let new_content = !candidates.is_empty();
                self.process_round(candidates, 2).await;
                self.render_pending();
                self.add_snippets();
                let remaining = intent::missing_sides(&self.report.documents, &sides);
                let filled = missing.iter().any(|side| !remaining.contains(side));
                self.report.notes.push(format!(
                    "round-2: new_body={} new_content={new_content} gap_filled={filled}",
                    self.bodies.len() > before
                ));
                for side in remaining {
                    self.report.notes.push(format!(
                        "evidence gap remains after rewrite: {side}; one rewrite maximum"
                    ));
                }
            }
            Err(error) => self
                .report
                .notes
                .push(format!("rewrite search failed: {error}")),
        }
    }
}
