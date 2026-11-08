use super::*;

impl AgentMemoryRepository {
    pub fn validate_citation_sources(
        &self,
        generation: u64,
        resource_id: &str,
        source_session_ids: &[String],
    ) -> Result<bool, AgentMemoryError> {
        if generation != self.generation()? || source_session_ids.is_empty() {
            return Ok(false);
        }
        let resource = self.inner.paths.resource_path(resource_id)?;
        if !resource.is_file() {
            return Ok(false);
        }
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(self.inner.paths.artifacts.join("manifest.json")).map_err(io_error)?,
        )
        .map_err(|_| AgentMemoryError::Corrupt("memory manifest is invalid".into()))?;
        if manifest
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            != Some(generation)
        {
            return Ok(false);
        }
        let sources = manifest
            .get("sources")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| AgentMemoryError::Corrupt("memory manifest has no sources".into()))?;
        let notes = manifest
            .get("notes")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| AgentMemoryError::Corrupt("memory manifest has no notes".into()))?;
        Ok(source_session_ids.iter().all(|source| {
            sources.iter().chain(notes).any(|candidate| {
                candidate
                    .get("sessionId")
                    .and_then(serde_json::Value::as_str)
                    == Some(source.as_str())
            })
        }))
    }

    /// The published summary, as it stands on disk.
    ///
    /// The generation counter belongs to the write pipeline and is carried here
    /// only as provenance. It is deliberately not an admission gate: a memory a
    /// user can see in the artifacts folder is a memory the model must be able
    /// to read, whether or not a consolidation pass has run since, and whether
    /// or not newer sources are still waiting for one. Gating the read on the
    /// writer meant that asking to remember something made every existing
    /// memory disappear until the next successful pass.
    ///
    /// An over-long summary is truncated rather than refused, for the same
    /// reason: this is a context budget, and the whole memory is not worth
    /// losing to it.
    pub fn load_context(&self) -> Result<Option<AgentMemoryContext>, AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let generation = self.published_generation()?;
        let path = self.inner.paths.resource_path("memory_summary.md")?;
        let summary = match fs::read_to_string(path) {
            Ok(summary) => summary,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        let summary = bound_text(summary.trim(), MAX_MEMORY_SUMMARY_BYTES);
        if summary.is_empty() {
            return Ok(None);
        }
        Ok(Some(AgentMemoryContext {
            generation,
            summary,
        }))
    }

    /// Everything published in the artifacts folder, as it stands.
    ///
    /// Size is reported, never used to hide a resource: a large `MEMORY.md` is
    /// listed and read back truncated, because a memory the model cannot see is
    /// worse than a memory it sees the first part of.
    pub fn list_resources(&self) -> Result<Vec<AgentMemoryResource>, AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let mut resources = Vec::new();
        for id in ["MEMORY.md", "memory_summary.md"] {
            let path = self.inner.paths.resource_path(id)?;
            if let Ok(metadata) = path.metadata() {
                if metadata.is_file() {
                    resources.push(AgentMemoryResource {
                        id: id.to_string(),
                        bytes: metadata.len() as usize,
                    });
                }
            }
        }
        let rollouts = self.inner.paths.artifacts.join("rollout_summaries");
        if rollouts.is_dir() {
            for entry in fs::read_dir(rollouts).map_err(io_error)? {
                let entry = entry.map_err(io_error)?;
                let metadata = entry.metadata().map_err(io_error)?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if metadata.is_file() && name.ends_with(".md") && !name.starts_with('.') {
                    resources.push(AgentMemoryResource {
                        id: format!("rollout_summaries/{name}"),
                        bytes: metadata.len() as usize,
                    });
                }
            }
        }
        resources.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(resources)
    }

    pub fn read_resource(
        &self,
        resource_id: &str,
        start_line: usize,
        maximum_lines: usize,
    ) -> Result<AgentMemoryReadResult, AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let generation = self.published_generation()?;
        if start_line == 0 || maximum_lines == 0 || maximum_lines > 500 {
            return Err(AgentMemoryError::InvalidInput(
                "memory read line bounds are invalid".into(),
            ));
        }
        let path = self.inner.paths.resource_path(resource_id)?;
        let bytes = fs::read(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AgentMemoryError::NotFound
            } else {
                io_error(error)
            }
        })?;
        let text = String::from_utf8(bytes)
            .map_err(|_| AgentMemoryError::Corrupt("memory resource is not UTF-8".into()))?;
        // A large resource is read up to the bound, not refused. The line range
        // the caller asked for is the real budget; this only stops one enormous
        // file from being loaded whole.
        let text = bound_text(&text, MAX_MEMORY_RESOURCE_BYTES);
        let lines = text.lines().collect::<Vec<_>>();
        let start = start_line.saturating_sub(1).min(lines.len());
        let end = start.saturating_add(maximum_lines).min(lines.len());
        Ok(AgentMemoryReadResult {
            generation,
            resource_id: resource_id.to_string(),
            start_line,
            end_line: end,
            content: lines[start..end].join("\n"),
            truncated: end < lines.len(),
        })
    }

    pub fn search(
        &self,
        query: &str,
        maximum_results: usize,
    ) -> Result<Vec<AgentMemorySearchMatch>, AgentMemoryError> {
        let query = query.trim();
        if query.is_empty() || query.len() > 512 {
            return Err(AgentMemoryError::InvalidInput(
                "memory search query must contain 1 to 512 bytes".into(),
            ));
        }
        let needle = query.to_lowercase();
        let mut matches = Vec::new();
        for resource in self.list_resources()? {
            let text = fs::read_to_string(self.inner.paths.resource_path(&resource.id)?)
                .map_err(io_error)?;
            for (index, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    matches.push(AgentMemorySearchMatch {
                        resource_id: resource.id.clone(),
                        line: index + 1,
                        excerpt: bound_string(line, 512),
                    });
                    if matches.len() >= maximum_results.clamp(1, MAX_MEMORY_SEARCH_RESULTS) {
                        return Ok(matches);
                    }
                }
            }
        }
        Ok(matches)
    }
}
