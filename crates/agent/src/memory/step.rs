//! What memory contributes to a sampling step, for the `memory` plugin: the
//! context the model is given, whether external content pollutes a
//! session's contribution, and the citations an answer carries.

use super::*;

impl AgentMemoryApplication {
    /// The published memory summary, read as it stands for every sampling
    /// step, while memory retrieval is enabled.
    pub(crate) fn load_prompt_context(&self) -> Result<Option<AgentMemoryContext>, String> {
        let memory = self.config.settings()?.memory;
        if !memory.enabled || !memory.use_memories {
            return Ok(None);
        }
        self.repository
            .load_context()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn excludes_external_context(&self) -> Result<bool, String> {
        let memory = self.config.settings()?.memory;
        Ok(memory.enabled && memory.exclude_external_context)
    }

    pub(crate) fn process_final_answer(
        &self,
        text: &str,
    ) -> Result<AgentProcessedMemoryAnswer, String> {
        let parsed = match hanafubuki_agent_memory::parse_memory_citations(text) {
            Ok(parsed) => parsed,
            Err(error) => {
                log::warn!("Ignoring malformed Agent memory citations: {error}");
                return Ok(AgentProcessedMemoryAnswer {
                    visible_text: text.to_string(),
                    citations: Vec::new(),
                });
            }
        };
        if parsed.citations.is_empty() {
            return Ok(AgentProcessedMemoryAnswer {
                visible_text: parsed.visible_text,
                citations: Vec::new(),
            });
        }
        if parsed.citations.iter().any(|citation| {
            !(matches!(
                citation.resource_id.as_str(),
                "MEMORY.md" | "memory_summary.md"
            ) || citation.resource_id.starts_with("rollout_summaries/"))
                || citation
                    .source_session_ids
                    .iter()
                    .any(|source| source.parse::<u64>().is_err())
        }) {
            return Ok(AgentProcessedMemoryAnswer {
                visible_text: text.to_string(),
                citations: Vec::new(),
            });
        }
        let generation = self
            .repository
            .generation()
            .map_err(|error| error.to_string())?;
        if parsed.citations.iter().any(|citation| {
            !self
                .repository
                .validate_citation_sources(
                    generation,
                    &citation.resource_id,
                    &citation.source_session_ids,
                )
                .unwrap_or(false)
        }) {
            return Ok(AgentProcessedMemoryAnswer {
                visible_text: text.to_string(),
                citations: Vec::new(),
            });
        }
        Ok(AgentProcessedMemoryAnswer {
            visible_text: parsed.visible_text,
            citations: parsed
                .citations
                .into_iter()
                .map(|citation| AgentMemoryCitationSnapshot {
                    generation,
                    resource_id: citation.resource_id,
                    start_line: citation.start_line,
                    end_line: citation.end_line,
                    note: citation.note,
                    source_session_ids: citation.source_session_ids,
                })
                .collect(),
        })
    }
}
