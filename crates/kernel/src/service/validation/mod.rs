use sha2::{Digest, Sha256};

use super::*;

mod activities;
mod config;
mod counters;
mod history;
mod persistence;
mod tool_pairs;

pub(in crate::service) use activities::{
    bounded_diagnostic, enforce_size, map_budget_error, map_no_progress_error,
    validate_bounded_identifier, validate_persisted_activities,
};
pub(in crate::service) use config::{
    assert_valid_limits, normalize_session_config, resolve_turn_items, validate_host_context,
    validate_session_config,
};
pub(in crate::service) use counters::validate_persisted_counters;
pub(in crate::service) use history::{
    validate_context_window_checkpoint, validate_persisted_compaction, validate_persisted_usage,
    validate_user_input_answers,
};
pub(in crate::service) use persistence::{
    validate_history_append, validate_permission_review_state, validate_persisted_history,
    validate_persisted_message_state,
};
pub(in crate::service) use tool_pairs::{
    interrupted_tool_outputs, normalized_tool_pairs_for_model, validate_complete_tool_pairs,
};
use tool_pairs::{skill_context_follows_tool_output, validate_skill_context_groups};

#[cfg(test)]
mod tests;
