//! How the interface names what is known of a model: where a fact came
//! from, and what a model list's entry reports.

use hanafubuki_agent::{ModelCatalogEntry, ModelMetadataSource};

use crate::history_cell::format_tokens_compact;

/// Where a fact came from, as the interface names it.
pub(crate) fn source_name(source: ModelMetadataSource) -> &'static str {
    match source {
        ModelMetadataSource::Configured => "configured",
        ModelMetadataSource::Learned => "learned",
        ModelMetadataSource::Reported => "reported",
        ModelMetadataSource::Catalog => "models.dev",
        ModelMetadataSource::BuiltIn => "built-in",
    }
}

/// What a listing's entry reports, for a model list: its window, and
/// `vision` and `reasoning` when it reports them (`200K · vision`); none
/// when it reports none of them.
pub(crate) fn entry_badges(entry: &ModelCatalogEntry) -> Option<String> {
    let badges: Vec<String> = entry
        .context_window
        .map(format_tokens_compact)
        .into_iter()
        .chain((entry.image_input == Some(true)).then(|| "vision".to_string()))
        .chain((entry.reasoning == Some(true)).then(|| "reasoning".to_string()))
        .collect();
    (!badges.is_empty()).then(|| badges.join(" · "))
}

#[cfg(test)]
mod tests;
