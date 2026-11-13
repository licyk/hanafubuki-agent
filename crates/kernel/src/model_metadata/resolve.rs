//! The precedence of the sources, applied per fact: configured, then
//! learned in the safe direction only, then reported, then models.dev's
//! catalogue, which only adds, then built-in.

use hanafubuki_agent_model_api::{
    ModelMetadata, ModelMetadataSource, ModelReasoningSupport, Sourced,
};

use super::state::RouteFacts;
use super::ConfiguredModelFacts;

/// What is known about a route from its `facts` (none when nothing is),
/// what the person `configured`, what models.dev's `catalog` says of its
/// model, and the built-in window of its model.
pub(super) fn resolve(
    facts: Option<&RouteFacts>,
    catalog: &ModelMetadata,
    configured: ConfiguredModelFacts,
    built_in_window: Option<u64>,
) -> ModelMetadata {
    let empty = RouteFacts::default();
    let RouteFacts { reported, learned } = facts.unwrap_or(&empty);
    ModelMetadata {
        context_window: limit(
            configured.context_window,
            &learned.context_window,
            &reported.context_window,
            &catalog.context_window,
            built_in_window,
        ),
        max_output_tokens: limit(
            configured.max_output_tokens,
            &learned.max_output_tokens,
            &reported.max_output_tokens,
            &catalog.max_output_tokens,
            None,
        ),
        reasoning: capability(
            configured.reasoning,
            &learned.reasoning,
            &reported.reasoning,
            &catalog.reasoning,
            |support| matches!(support, ModelReasoningSupport::Supported { .. }),
        ),
        image_input: capability(
            configured.image_input,
            &learned.image_input,
            &reported.image_input,
            &catalog.image_input,
            |reads_images| *reads_images,
        ),
        // Only a refusal teaches the rung, and only ever a lower one than
        // the protocol's, which the session applies.
        structured_output: learned.structured_output.clone(),
    }
}

/// A limit: a learned one only while it is smaller than the one the other
/// sources give (or they give none), since a refusal proves a limit is no
/// larger, never that it is larger.
fn limit(
    configured: Option<u64>,
    learned: &Option<Sourced<u64>>,
    reported: &Option<Sourced<u64>>,
    catalog: &Option<Sourced<u64>>,
    built_in: Option<u64>,
) -> Option<Sourced<u64>> {
    if let Some(value) = configured {
        return Some(unobserved(value, ModelMetadataSource::Configured));
    }
    let known = reported
        .clone()
        .or_else(|| catalog.clone())
        .or_else(|| built_in.map(|value| unobserved(value, ModelMetadataSource::BuiltIn)));
    match learned {
        Some(learned)
            if known
                .as_ref()
                .is_none_or(|known| learned.value < known.value) =>
        {
            Some(learned.clone())
        }
        _ => known,
    }
}

/// A capability: a learned "no" beats a reported "yes" (the Provider's
/// refusal is evidence, its listing a claim), and a catalog only adds a
/// "yes" where nothing else is known.
fn capability<T: Clone>(
    configured: Option<T>,
    learned: &Option<Sourced<T>>,
    reported: &Option<Sourced<T>>,
    catalog: &Option<Sourced<T>>,
    is_yes: impl Fn(&T) -> bool,
) -> Option<Sourced<T>> {
    if let Some(value) = configured {
        return Some(unobserved(value, ModelMetadataSource::Configured));
    }
    learned
        .as_ref()
        .filter(|learned| !is_yes(&learned.value))
        .or(reported.as_ref())
        .or(catalog.as_ref().filter(|catalog| is_yes(&catalog.value)))
        .cloned()
}

fn unobserved<T>(value: T, source: ModelMetadataSource) -> Sourced<T> {
    Sourced {
        value,
        source,
        observed_at_ms: 0,
    }
}
