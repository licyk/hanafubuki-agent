//! The `review` plugin: Codex's `/review` over the session driver. A review
//! is a durable child session of origin `Review`, started empty with
//! Codex's rubric as a prompt fragment and the target's request as its
//! first message, kept read-only by an approval rule; its final answer is
//! read as findings, and the reviewed conversation is told how it ended.
//! The plugin provides [`ReviewService`] under its name, which the facade's
//! review calls resolve per call; with the plugin off, those calls are
//! unavailable, and a review session neither sees the rubric nor is kept
//! read-only.

mod output;
mod read_only;
mod request;
mod rubric;
mod service;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ApprovalRuleRegistry, BlockingService, ContextRendererRegistry, PromptContributorRegistry,
    Register,
};
use hanafubuki_agent_worktree::GitQueries;
use kardia_core::{Context, Inject, PluginDef, Service};
use serde::{Deserialize, Serialize};

pub use self::output::{
    AgentReviewCodeLocation, AgentReviewFinding, AgentReviewLineRange, AgentReviewOutput,
};
use self::read_only::ReviewIsReadOnly;
use self::rubric::{ReviewRubricPrompt, REVIEW_INSTRUCTIONS_RENDERER};
pub(crate) use self::service::ReviewService;
use crate::session_driver::AgentSessionDriver;

/// The plugin's name, under which it provides its [`ReviewService`].
pub(crate) const REVIEW_PLUGIN: &str = "review";

/// What a review looks at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentReviewTarget {
    /// The working tree's staged, unstaged and untracked changes.
    UncommittedChanges,
    /// What `HEAD` adds to `branch`, from their merge base.
    BaseBranch { branch: String },
    /// What one commit changed; `title` is its subject, when known.
    Commit { sha: String, title: Option<String> },
    /// The user's own request, as the review's first message.
    Custom { instructions: String },
}

/// What the review targets are chosen from: the working directory's
/// repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewTargets {
    /// The local branches, the default one (`main`, else `master`) first.
    pub branches: Vec<String>,
    /// `None` while `HEAD` is detached.
    pub current_branch: Option<String>,
    /// The latest 100 commits of `HEAD`, newest first.
    pub commits: Vec<AgentReviewCommit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewCommit {
    pub sha: String,
    /// The first line of its message.
    pub subject: String,
}

/// Registers the rubric, the read-only rule and the [`ReviewService`] over
/// `driver`, with `git` for the targets.
pub(crate) fn review_plugin(driver: AgentSessionDriver, git: GitQueries) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let driver = driver.clone();
        let git = git.clone();
        async move {
            let service = Arc::new(ReviewService {
                driver,
                git,
                blocking: ctx.get::<BlockingService>()?.arc().clone(),
            });
            ctx.get::<ApprovalRuleRegistry>()?
                .register(Arc::new(ReviewIsReadOnly))?;
            ctx.get::<PromptContributorRegistry>()?
                .register(Arc::new(ReviewRubricPrompt {
                    service: service.clone(),
                }))?;
            ctx.get::<ContextRendererRegistry>()?
                .register(REVIEW_INSTRUCTIONS_RENDERER)?;
            ctx.provide_arc_as(REVIEW_PLUGIN, service)?;
            Ok(())
        }
    })
    .name(REVIEW_PLUGIN)
    .provides([REVIEW_PLUGIN])
    .inject(
        Inject::new()
            .with(ApprovalRuleRegistry::NAME)
            .with(PromptContributorRegistry::NAME)
            .with(ContextRendererRegistry::NAME)
            .with(BlockingService::NAME),
    )
}

#[cfg(test)]
mod tests;
