use std::path::PathBuf;
use std::sync::Arc;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_worktree::{Worktree, WorktreeError, WorktreeManager};
use kardia_core::{Context, Inject, PluginDef, Service};

/// The plugin's name, under which it provides its [`WorktreeService`].
pub const WORKTREE_PLUGIN: &str = "worktree";

/// Provides the [`WorktreeService`] over `worktrees`, which the application
/// builds over its own `ExecutorManager`.
pub fn worktree_plugin(worktrees: WorktreeManager) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let worktrees = worktrees.clone();
        async move {
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            ctx.provide_arc_as(
                WORKTREE_PLUGIN,
                Arc::new(WorktreeService {
                    worktrees,
                    blocking,
                }),
            )?;
            Ok(())
        }
    })
    .name(WORKTREE_PLUGIN)
    .provides([WORKTREE_PLUGIN])
    .inject(Inject::new().with(BlockingService::NAME))
}

/// What the `worktree` row provides: [`WorktreeManager`]'s operations on
/// the blocking pool, since each runs `git` to its end. A failure is the
/// operation's message.
pub struct WorktreeService {
    worktrees: WorktreeManager,
    blocking: Arc<BlockingService>,
}

impl WorktreeService {
    /// See [`WorktreeManager::create`].
    pub async fn create(&self, source: PathBuf) -> Result<Worktree, String> {
        self.run("agent worktree create", move |worktrees| {
            worktrees.create(&source)
        })
        .await
    }

    /// See [`WorktreeManager::list`].
    pub async fn list(&self, source: PathBuf) -> Result<Vec<Worktree>, String> {
        self.run("agent worktree list", move |worktrees| {
            worktrees.list(&source)
        })
        .await
    }

    /// See [`WorktreeManager::bind`].
    pub async fn bind(&self, path: PathBuf, session: String) -> Result<(), String> {
        self.run("agent worktree bind", move |worktrees| {
            worktrees.bind(&path, &session)
        })
        .await
    }

    /// See [`WorktreeManager::remove`].
    pub async fn remove(&self, source: PathBuf, path: PathBuf) -> Result<(), String> {
        self.run("agent worktree remove", move |worktrees| {
            worktrees.remove(&source, &path)
        })
        .await
    }

    async fn run<T: Send + 'static>(
        &self,
        label: &'static str,
        operation: impl FnOnce(&WorktreeManager) -> Result<T, WorktreeError> + Send + 'static,
    ) -> Result<T, String> {
        let worktrees = self.worktrees.clone();
        self.blocking
            .run(label, move || operation(&worktrees))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }
}
