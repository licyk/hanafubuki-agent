use super::*;
use crate::snapshot::{self, ShellSnapshot};
use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};

#[derive(PartialEq, Eq)]
pub(super) struct SnapshotKey {
    cwd: PathKey,
    program: PathKey,
    environment: std::collections::BTreeMap<String, String>,
    settings: crate::ShellEnvironment,
    sandbox: hanafubuki_agent_sandbox::SandboxSettings,
    unsandboxed: bool,
    overlay: HashMap<String, Option<String>>,
}

pub(super) struct CachedSnapshot {
    key: SnapshotKey,
    snapshot: Option<Arc<ShellSnapshot>>,
}

impl AgentExecutionService {
    pub(super) fn restore_shell_snapshot(
        &self,
        scope: &AgentExecutionScope,
        request: &ExecCommandRequest,
        plan: &mut ResolvedCommandPlan,
        cancellation: &CancellationToken,
    ) -> Result<(), AgentExecutionError> {
        if !snapshot::enabled(request, plan) {
            return Ok(());
        }
        let path_key = |path: &PathBuf| {
            ResolvedPath::resolve(path)
                .map(|path| path.key().clone())
                .map_err(|error| AgentExecutionError::Resolution(error.to_string()))
        };
        let mut settings = request.shell_environment.clone();
        settings.context = None;
        let mut environment = plan.environment.clone();
        environment.retain(|key, _| {
            !crate::environment::RUNTIME_IDENTITY_KEYS
                .iter()
                .any(|name| name.eq_ignore_ascii_case(key))
        });
        let key = SnapshotKey {
            cwd: path_key(&plan.cwd)?,
            program: path_key(&PathBuf::from(&plan.command[0]))?,
            overlay: self.inner.process.environment_overlay(),
            environment,
            settings,
            sandbox: request.sandbox.clone(),
            unsandboxed: request.sandbox_escalation.is_some(),
        };
        let cached = self
            .inner
            .shell_snapshots
            .lock()
            .get(&scope.session_id())
            .filter(|entry| entry.key == key)
            .map(|entry| entry.snapshot.clone());
        let snapshot = match cached {
            Some(snapshot) => snapshot,
            None => {
                let snapshot = self.capture_shell_snapshot(scope, request, plan, cancellation)?;
                self.inner.shell_snapshots.lock().insert(
                    scope.session_id(),
                    CachedSnapshot {
                        key,
                        snapshot: snapshot.clone(),
                    },
                );
                snapshot
            }
        };
        if let Some(snapshot) = snapshot {
            snapshot.apply(request, plan)?;
        }
        Ok(())
    }

    fn capture_shell_snapshot(
        &self,
        scope: &AgentExecutionScope,
        request: &ExecCommandRequest,
        plan: &ResolvedCommandPlan,
        cancellation: &CancellationToken,
    ) -> Result<Option<Arc<ShellSnapshot>>, AgentExecutionError> {
        let Some(kind) = snapshot::kind(&plan.command[0]) else {
            return Ok(None);
        };
        let script = snapshot::capture_script(kind);
        #[cfg(windows)]
        let script = crate::msys::command_line_script(&script)?;
        let command =
            self.sandboxed(vec![plan.command[0].clone(), "-lc".into(), script], request)?;
        let capture = ResolvedCommandPlan {
            command,
            cwd: plan.cwd.clone(),
            environment: plan.environment.clone(),
            timeout_ms: 10_000,
            output_bytes_cap: 256 * 1024,
            io_mode: ExecutorIoMode::Pipe,
            cols: None,
            rows: None,
            interactive: false,
        };
        let started = self.start_plan(
            scope.clone(),
            capture,
            AgentExecutionKind::Command,
            None,
            cancellation,
        )?;
        let result = (|| {
            let waited = self.wait(scope, started.execution_id, 15_000, cancellation)?;
            if !waited.completed {
                return Ok(None);
            }
            let receipt = self.receipt_for_scope(scope, started.execution_id)?;
            let output = self
                .inner
                .process
                .read(&receipt, None, Some(256 * 1024))
                .map_err(AgentExecutionError::Process)?;
            if output.exit_code != Some(0) || output.truncated {
                return Ok(None);
            }
            let bytes: Vec<_> = output
                .chunks
                .into_iter()
                .filter(|chunk| {
                    chunk.stream == hanafubuki_agent_process::executor::OutputStream::Stdout
                })
                .flat_map(|chunk| chunk.data)
                .collect();
            Ok(ShellSnapshot::parse(&bytes).map(Arc::new))
        })();
        self.stop_confirmed(scope, started.execution_id)?;
        result
    }
}
