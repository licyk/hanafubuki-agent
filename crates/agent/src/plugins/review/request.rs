// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/prompts/src/review_request.rs` (the first message of a review
// per target, and the target as the user reads it) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the merge base is looked up by the caller;
// the texts are formatted rather than rendered from templates; the hint also
// names the review session.

//! What a review is asked: the first message of the review session for each
//! target, and the target in a few words.

use super::AgentReviewTarget;

const UNCOMMITTED_PROMPT: &str = "Review the current code changes (staged, unstaged, and untracked files) and provide prioritized findings.";

/// The first message of a review of `target`; `merge_base` is the merge
/// base of `HEAD` with a base branch, when it was found.
pub(super) fn review_prompt(target: &AgentReviewTarget, merge_base: Option<&str>) -> String {
    match target {
        AgentReviewTarget::UncommittedChanges => UNCOMMITTED_PROMPT.to_string(),
        AgentReviewTarget::BaseBranch { branch } => match merge_base {
            Some(merge_base_sha) => format!(
                "Review the code changes against the base branch '{branch}'. The merge base commit for this comparison is {merge_base_sha}. Run `git diff {merge_base_sha}` to inspect the changes relative to {branch}. Provide prioritized, actionable findings."
            ),
            None => format!(
                "Review the code changes against the base branch '{branch}'. Start by finding the merge diff between the current branch and {branch}'s upstream e.g. (`git merge-base HEAD \"$(git rev-parse --abbrev-ref \"{branch}@{{upstream}}\")\"`), then run `git diff` against that SHA to see what changes we would merge into the {branch} branch. Provide prioritized, actionable findings."
            ),
        },
        AgentReviewTarget::Commit {
            sha,
            title: Some(title),
        } => format!(
            "Review the code changes introduced by commit {sha} (\"{title}\"). Provide prioritized, actionable findings."
        ),
        AgentReviewTarget::Commit { sha, title: None } => format!(
            "Review the code changes introduced by commit {sha}. Provide prioritized, actionable findings."
        ),
        AgentReviewTarget::Custom { instructions } => instructions.trim().to_string(),
    }
}

/// The target as the user reads it.
pub(super) fn user_facing_hint(target: &AgentReviewTarget) -> String {
    match target {
        AgentReviewTarget::UncommittedChanges => "current changes".to_string(),
        AgentReviewTarget::BaseBranch { branch } => format!("changes against '{branch}'"),
        AgentReviewTarget::Commit { sha, title } => {
            let short_sha = sha.chars().take(7).collect::<String>();
            match title {
                Some(title) => format!("commit {short_sha}: {title}"),
                None => format!("commit {short_sha}"),
            }
        }
        AgentReviewTarget::Custom { instructions } => instructions.trim().to_string(),
    }
}
