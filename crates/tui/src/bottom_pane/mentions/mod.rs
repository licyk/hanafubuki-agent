// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Unified @ mentions, with the same picker layout and keyboard filters as Codex.
mod candidate;
mod filter;
mod footer;
mod layout;
mod popup;
mod render;
mod search_mode;
mod style;
mod tabs;

pub(crate) use candidate::{Candidate, MentionType, Selection};
pub(crate) use popup::Popup;
