//! Repository fixtures the memory suites share.

use std::fs;

use tempfile::TempDir;

use hanafubuki_agent_memory::*;

pub fn repository() -> (TempDir, AgentMemoryRepository) {
    let temporary = TempDir::new().unwrap();
    let repository =
        AgentMemoryRepository::open(AgentMemoryPaths::under(temporary.path().join("memory")))
            .unwrap();
    (temporary, repository)
}

pub fn sync_enabled(repository: &AgentMemoryRepository, session: &str, source_revision: u64) {
    repository
        .sync_source_state(
            session,
            AgentMemoryContributionMode::Enabled,
            1,
            source_revision,
        )
        .unwrap();
}

pub fn stage_one(raw: &str, slug: &str) -> StageOneResult {
    StageOneResult {
        raw_memory: raw.into(),
        rollout_summary: format!("summary for {raw}"),
        rollout_slug: slug.into(),
    }
}

pub fn publish_workspace(
    repository: &AgentMemoryRepository,
    lease: &MemoryLease,
    inputs: &[(String, u64, StageOneResult)],
    memory: &str,
    summary: &str,
    now: u64,
) -> u64 {
    let workspace = repository
        .prepare_consolidation_workspace(lease.source_revision + 1, inputs)
        .unwrap();
    fs::write(workspace.path.join("MEMORY.md"), memory).unwrap();
    fs::write(workspace.path.join("memory_summary.md"), summary).unwrap();
    repository
        .publish_consolidation_workspace(lease, &workspace, now)
        .unwrap()
}

pub fn published_single_source() -> (TempDir, AgentMemoryRepository) {
    let (temporary, repository) = repository();
    sync_enabled(&repository, "baseline", 1);
    let source = repository
        .claim_source("baseline", 1, 100, 10_000)
        .unwrap()
        .unwrap();
    repository
        .complete_source(&source, Some(stage_one("baseline memory", "baseline")), 101)
        .unwrap();
    let lease = repository
        .claim_consolidation(102, 10_000)
        .unwrap()
        .unwrap();
    let inputs = repository.consolidation_inputs(32).unwrap();
    publish_workspace(
        &repository,
        &lease,
        &inputs,
        "# Memory\n\nBaseline memory.\n",
        "v1\n\nBaseline memory.\n",
        103,
    );
    (temporary, repository)
}
