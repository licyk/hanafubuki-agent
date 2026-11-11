//! The order of reasoning efforts, and the effort a request carries for a
//! route whose reasoning is known.
//!
//! The ladder is `minimal < low < medium < high < xhigh < max`; `light`,
//! `balanced` and `deep` (Hoshimi's names) stand on `low`, `medium` and
//! `high`. An effort off the ladder has no rank.

use crate::{ModelReasoningEffort, ModelReasoningSupport};

/// The effort a request carries when the session is configured with
/// `configured` and the route's reasoning is `support` (none: unknown).
///
/// - No configured effort: none, so the Provider's default applies.
/// - Unknown, or supported with no levels reported: the configured effort.
/// - Unsupported: none, whatever is configured.
/// - Supported at listed levels: the configured effort when listed, else
///   its alias when listed (`minimal`, `light` → `low`, `balanced` →
///   `medium`, `deep` → `high`, `xhigh` → `max`), else the listed level
///   nearest on the ladder (a tie goes to the lower), else none when
///   neither it nor any listed level has a rank.
pub fn request_reasoning_effort(
    configured: Option<&ModelReasoningEffort>,
    support: Option<&ModelReasoningSupport>,
) -> Option<ModelReasoningEffort> {
    let configured = configured?;
    match support {
        None => Some(configured.clone()),
        Some(ModelReasoningSupport::Unsupported) => None,
        Some(ModelReasoningSupport::Supported { efforts, .. }) if efforts.is_empty() => {
            Some(configured.clone())
        }
        Some(ModelReasoningSupport::Supported { efforts, .. }) => narrowed(configured, efforts),
    }
}

/// `effort`'s place on the ladder, from 0 (`minimal`) to 5 (`max`).
pub fn reasoning_effort_rank(effort: &ModelReasoningEffort) -> Option<u8> {
    Some(match effort.as_str() {
        "minimal" => 0,
        "low" | "light" => 1,
        "medium" | "balanced" => 2,
        "high" | "deep" => 3,
        "xhigh" => 4,
        "max" => 5,
        _ => return None,
    })
}

fn narrowed(
    configured: &ModelReasoningEffort,
    levels: &[ModelReasoningEffort],
) -> Option<ModelReasoningEffort> {
    if levels.contains(configured) {
        return Some(configured.clone());
    }
    if let Some(alias) = alias(configured).filter(|alias| levels.contains(alias)) {
        return Some(alias);
    }
    let rank = reasoning_effort_rank(configured)?;
    levels
        .iter()
        .filter_map(|level| Some((reasoning_effort_rank(level)?, level)))
        .min_by_key(|(level_rank, _)| (level_rank.abs_diff(rank), *level_rank))
        .map(|(_, level)| level.clone())
}

fn alias(effort: &ModelReasoningEffort) -> Option<ModelReasoningEffort> {
    let alias = match effort.as_str() {
        "minimal" | "light" => "low",
        "balanced" => "medium",
        "deep" => "high",
        "xhigh" => "max",
        _ => return None,
    };
    ModelReasoningEffort::parse(alias).ok()
}

#[cfg(test)]
mod tests;
