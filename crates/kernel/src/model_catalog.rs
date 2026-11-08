//! The context window of the model a session samples with.

use std::collections::HashMap;

use hanafubuki_agent_model_api::ModelCatalogEntry;
use parking_lot::Mutex;

/// The context window per model: the session's configured window when it
/// has one, else what the provider's model listing reported, else the
/// built-in table of well-known models. With none of the three a session has
/// no window, so no automatic compaction.
///
/// The provider's listing is what `AgentService::list_provider_models`
/// last fetched in this run (the Settings model picker); a session is given
/// its window when it is created or its model is selected.
#[derive(Default)]
pub(crate) struct ModelCatalog {
    /// Per provider base URL, the windows its last listing reported.
    reported: Mutex<HashMap<String, HashMap<String, u64>>>,
}

impl ModelCatalog {
    pub(crate) fn record_listing(&self, base_url: &str, entries: &[ModelCatalogEntry]) {
        let windows = entries
            .iter()
            .filter_map(|entry| Some((entry.id.clone(), entry.context_window?)))
            .collect();
        self.reported.lock().insert(base_url.to_string(), windows);
    }

    pub(crate) fn context_window(
        &self,
        configured: Option<u64>,
        base_url: &str,
        model: &str,
    ) -> Option<u64> {
        configured
            .or_else(|| self.reported.lock().get(base_url)?.get(model).copied())
            .or_else(|| built_in_context_window(model))
    }
}

/// The fallback for a provider that reports no window, by model id prefix;
/// the longest matching prefix wins. Kept to a few well-known families and
/// conservative where a family's models differ: a window set too small only
/// compacts early, one set too large fails a request first.
const BUILT_IN_CONTEXT_WINDOWS: &[(&str, u64)] = &[
    ("gpt-4o", 128_000),
    ("gpt-4.1", 1_047_576),
    // The input context Codex uses for its GPT-5 family.
    ("gpt-5", 272_000),
    ("o3", 200_000),
    ("o4-mini", 200_000),
    // Claude before 4.6 and Haiku 4.5 have 200K; the later models 1M.
    ("claude-", 200_000),
    ("claude-opus-4-6", 1_000_000),
    ("claude-opus-4-7", 1_000_000),
    ("claude-opus-4-8", 1_000_000),
    ("claude-sonnet-4-6", 1_000_000),
    ("claude-opus-5", 1_000_000),
    ("claude-sonnet-5", 1_000_000),
    ("claude-haiku-5", 1_000_000),
    ("claude-fable-5", 1_000_000),
    // Every Gemini 2.5 model's input token limit.
    ("gemini-2.5", 1_048_576),
];

/// `model`'s window in the built-in table, ignoring case and a routing
/// prefix such as `openai/`.
fn built_in_context_window(model: &str) -> Option<u64> {
    let model = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    BUILT_IN_CONTEXT_WINDOWS
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, tokens)| *tokens)
}

#[cfg(test)]
mod tests;
