//! A session's hierarchical compression as a person reads it: the status
//! panel and the cache report, read without changing the session. The
//! nudge the panel shows is decided again on a copy of the state, as
//! billion-context's `/acp` does, so a fold since the last request shows.

use hanafubuki_agent_compression::{
    build_status_panel, count_message_tokens, CacheDetail, StatusPanelInput,
};

use super::tools::cache_report;
use super::*;
use crate::AgentCompressionStatus;

impl AgentService {
    /// `session_id`'s hierarchical compression tuned by `settings`
    /// (`[compaction.hierarchical]`), or none when the session neither runs
    /// `hierarchical` nor kept a state from it. Nothing is committed. The
    /// record's lock is held only to copy what the panel reads.
    pub fn compression_status(
        &self,
        session_id: AgentSessionId,
        settings: &AgentHierarchicalCompression,
    ) -> Result<Option<AgentCompressionStatus>, AgentError> {
        let cell = self.session(session_id)?;
        let read = {
            let record = cell.lock_after_persistence_blocking()?;
            StatusRead::of(&record, settings)
        };
        let Some(read) = read else {
            return Ok(None);
        };
        let now_ms = now_millis_u64();
        catch_panic(|| read.status(now_ms))
            .map(Some)
            .map_err(|message| {
                AgentError::Internal(format!(
                    "the compression status could not be read: {message}"
                ))
            })
    }
}

/// What the status reads of the record, copied under its lock.
struct StatusRead {
    history: Arc<Vec<Arc<AgentHistoryItem>>>,
    state: AgentCompressionState,
    config: hanafubuki_agent_compression::Config,
    content_store: Option<AgentContentStore>,
    token_count: (u64, TokenCountSource),
    usage: Option<AgentModelUsage>,
    utc_offset_seconds: i32,
}

impl StatusRead {
    fn of(record: &SessionRecord, settings: &AgentHierarchicalCompression) -> Option<Self> {
        let hierarchical = record.settings.config.compaction_strategy
            == crate::AgentCompactionStrategy::Hierarchical;
        let state = record.runtime.compression.as_deref();
        if state.is_none() && !hierarchical {
            return None;
        }
        let window = record.settings.config.context_window_tokens.unwrap_or(0);
        let history = record.conversation.history.clone();
        Some(Self {
            token_count: token_count(record, &history),
            history,
            state: state.cloned().unwrap_or_default(),
            config: settings.config(window),
            content_store: record.runtime.compression_runtime.content_store().cloned(),
            usage: record.runtime.usage,
            utc_offset_seconds: settings.utc_offset_seconds,
        })
    }

    fn status(self, now_ms: u64) -> AgentCompressionStatus {
        let log = MessageLog::new(&self.history);
        let (token_count, source) = self.token_count;
        let content_store = self.content_store.as_ref().map(AgentContentStore::stats);
        let (output, _) = process_turn_with(
            TurnInput {
                messages: &log.messages,
                state: self.state.clone(),
                config: &self.config,
                token_count,
                token_count_source: Some(source),
                render: RenderStrategy::None,
                clock: None,
            },
            TurnExtras {
                content_store: self.content_store.unwrap_or_default(),
                filters: &[],
            },
        );
        let panel = build_status_panel(&StatusPanelInput {
            version: None,
            token_count,
            system_prompt_tokens: 0,
            state: &self.state,
            nudge: Some(&output.nudge),
            model_context_limit: self.config.model_context_limit,
            unpruned_tokens: Some(log.messages.iter().map(count_message_tokens).sum()),
            cache_usages: None,
        });
        AgentCompressionStatus {
            panel,
            cache_report: cache_report(
                self.usage,
                &self.state,
                CacheDetail::Full,
                now_ms,
                self.utc_offset_seconds,
            ),
            stats: self.state.stats.clone(),
            content_store,
        }
    }
}
