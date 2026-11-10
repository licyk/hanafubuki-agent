//! A guest's process: started through the Agent's `ExecutorManager`, in the
//! instance's owner group, as a reserved pipe executor whose pipes the
//! guest bridge reads and writes; confined by the sandbox's plugin profile;
//! with a cleared environment; and only while its package still has the
//! content recorded at install.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::background_worker::spawn_contained_thread;
use hanafubuki_agent_foundation::module_faults::FaultOwner;
use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::{content_hash, WorkspaceAccess};
use hanafubuki_agent_process::executor::manager::{
    dispose_reserved_executor, release_executor_reservation, reserve_executor_id,
    reserved_executor_exists, start_reserved_pipe_executor, terminate_reserved_executor,
    wait_reserved_executor, ReservedExecutorReceipt,
};
use hanafubuki_agent_process::executor::{
    EnvironmentInheritance, ExecutorIoMode, ExecutorManager, ExecutorStartRequest,
};
use hanafubuki_agent_process::ownership::ProcessOwnerId;
use hanafubuki_agent_sandbox::{Sandbox, SandboxPolicy};
use kardia_hmr::{GuestChild, GuestCommand, GuestProcess, Spawner};

use crate::package::GuestPackage;

/// The roots of the Agent's workspace, read at each start.
pub type WorkspaceRoots = Arc<dyn Fn() -> Vec<PathBuf> + Send + Sync>;

/// How long one wait for a process's end lasts before it is asked again.
const WAIT_STEP: Duration = Duration::from_millis(500);

static NEXT_GUEST: AtomicU64 = AtomicU64::new(1);

/// What every guest's start shares.
pub(crate) struct SpawnContext {
    /// The instance's executor, in its owner group.
    pub(crate) executor: ExecutorManager,
    /// `None`: guests run unconfined (tests only).
    pub(crate) sandbox: Option<Sandbox>,
    pub(crate) workspace: WorkspaceRoots,
    /// The bound on each export (`apply`, a call, `dispose`) and the
    /// handshake: past it the process is killed.
    pub(crate) call_timeout: Duration,
}

/// Starts the processes of one package's guest.
pub(crate) struct GuestSpawner {
    package: GuestPackage,
    context: Arc<SpawnContext>,
}

/// A prepared start: the command line (confined), the environment and the
/// working directory.
struct Launch {
    argv: Vec<String>,
    env: HashMap<String, Option<String>>,
    cwd: PathBuf,
}

impl GuestSpawner {
    pub(crate) fn new(package: GuestPackage, context: Arc<SpawnContext>) -> Self {
        Self { package, context }
    }

    fn launch(&self) -> Result<Launch, String> {
        let package = &self.package;
        check_content(package)?;
        std::fs::create_dir_all(&package.data_root)
            .map_err(|error| format!("its data directory cannot be created: {error}"))?;
        let root = resolve(&package.root, "its package directory")?;
        let data = resolve(&package.data_root, "its data directory")?;
        let (first, rest) = package
            .runtime
            .command
            .split_first()
            .ok_or("its runtime has no command")?;
        let program = if package.runtime.interpreter {
            interpreter(first)?
        } else {
            in_package(&root, first)?
        };
        let mut argv = vec![program.native_text().to_string()];
        for argument in rest {
            argv.push(match argument.starts_with("./") {
                true => in_package(&root, argument)?.native_text().to_string(),
                false => argument.clone(),
            });
        }
        let mut readable_roots = vec![root.native().to_path_buf()];
        if package.runtime.interpreter {
            readable_roots.push(program.native().to_path_buf());
        }
        let argv = match &self.context.sandbox {
            Some(sandbox) => sandbox
                .wrap(&self.profile(readable_roots, &data), argv)
                .map_err(|error| error.to_string())?,
            None => argv,
        };
        Ok(Launch {
            argv,
            env: environment(package, &root, &data),
            cwd: data.native().to_path_buf(),
        })
    }

    /// The plugin profile: the package read-only, its data directory
    /// writable, the workspace as granted, the network if granted.
    fn profile(&self, mut readable_roots: Vec<PathBuf>, data: &ResolvedPath) -> SandboxPolicy {
        let package = &self.package;
        let granted = package.granted.clone().unwrap_or_default();
        let mut writable_roots = vec![data.native().to_path_buf()];
        let workspace = (self.context.workspace)();
        match package.permissions.workspace.min(granted.workspace) {
            WorkspaceAccess::None => {}
            WorkspaceAccess::Read => readable_roots.extend(workspace),
            WorkspaceAccess::Write => writable_roots.extend(workspace),
        }
        SandboxPolicy::Plugin {
            readable_roots,
            writable_roots,
            network_access: package.permissions.network && granted.network,
        }
    }
}

impl Spawner for GuestSpawner {
    /// Spawns the package's command: the bridge's `command` names only its
    /// program, for messages.
    fn spawn(&self, _command: &GuestCommand) -> io::Result<GuestProcess> {
        let launch = self.launch().map_err(io::Error::other)?;
        start(&self.context.executor, &self.package, launch).map_err(io::Error::other)
    }
}

/// The package still has the content recorded at install, or its guest
/// does not start.
fn check_content(package: &GuestPackage) -> Result<(), String> {
    let recorded = package
        .content_hash
        .as_deref()
        .ok_or("its install recorded no content hash; install it again")?;
    let current = content_hash(&package.root)
        .map_err(|error| format!("its package cannot be hashed: {error}"))?;
    if current != recorded {
        return Err(
            "its package changed since it was installed; it does not start until it is installed again"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) fn resolve(path: &Path, what: &str) -> Result<ResolvedPath, String> {
    ResolvedPath::resolve(path).map_err(|error| format!("{what} does not resolve: {error}"))
}

/// A path in the package (`./…`), which must stay inside it.
pub(crate) fn in_package(root: &ResolvedPath, path: &str) -> Result<ResolvedPath, String> {
    let relative = path
        .strip_prefix("./")
        .ok_or_else(|| format!("`{path}` is not a path in the package (./…)"))?;
    let requested = root
        .join_request(Path::new(relative))
        .map_err(|error| format!("`{path}`: {error}"))?;
    let resolved =
        ResolvedPath::resolve(requested).map_err(|error| format!("`{path}`: {error}"))?;
    if !resolved.starts_with(root) {
        return Err(format!("`{path}` leads outside the package"));
    }
    Ok(resolved)
}

/// An interpreter outside the package: an absolute path, or a name found
/// on the Agent's `PATH`.
pub(crate) fn interpreter(program: &str) -> Result<ResolvedPath, String> {
    let path = Path::new(program);
    if path.is_absolute() {
        return resolve(path, "its interpreter");
    }
    let names: Vec<String> = match cfg!(windows) && path.extension().is_none() {
        true => vec![format!("{program}.exe"), program.to_string()],
        false => vec![program.to_string()],
    };
    let search = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&search)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .filter_map(|candidate| ResolvedPath::resolve(candidate).ok())
        .find(|candidate| std::fs::metadata(candidate.system()).is_ok_and(|meta| meta.is_file()))
        .ok_or_else(|| format!("its interpreter `{program}` is not on the PATH"))
}

/// Nothing of the Agent's environment but the variables the user granted,
/// with `PLUGIN_ROOT` and `PLUGIN_DATA` (on Windows also `SystemRoot`,
/// without which a process cannot load the system's libraries).
fn environment(
    package: &GuestPackage,
    root: &ResolvedPath,
    data: &ResolvedPath,
) -> HashMap<String, Option<String>> {
    let granted = package.granted.clone().unwrap_or_default();
    let mut env: HashMap<String, Option<String>> = package
        .permissions
        .env
        .iter()
        .filter(|name| granted.env.contains(name))
        .filter_map(|name| Some((name.clone(), Some(std::env::var(name).ok()?))))
        .collect();
    if cfg!(windows) {
        if let Ok(system_root) = std::env::var("SystemRoot") {
            env.insert("SystemRoot".to_string(), Some(system_root));
        }
    }
    env.insert(
        "PLUGIN_ROOT".to_string(),
        Some(root.native_text().to_string()),
    );
    env.insert(
        "PLUGIN_DATA".to_string(),
        Some(data.native_text().to_string()),
    );
    env
}

/// Reserves the process's owner, then starts it with its pipes handed out.
fn start(
    executor: &ExecutorManager,
    package: &GuestPackage,
    launch: Launch,
) -> Result<GuestProcess, String> {
    let executor_id = format!(
        "plugin-{}-{}",
        package.id.key(),
        NEXT_GUEST.fetch_add(1, Ordering::Relaxed)
    );
    let owner = ProcessOwnerId::generate();
    let reservation = reserve_executor_id(executor, &executor_id, &owner)?;
    let request = ExecutorStartRequest {
        executor_id: Some(executor_id),
        command: launch.argv,
        cwd: Some(launch.cwd),
        environment_inheritance: EnvironmentInheritance::Clear,
        env: launch.env,
        io_mode: ExecutorIoMode::Pipe,
        stream_stdin: true,
        stream_output: true,
        cols: None,
        rows: None,
        timeout_ms: None,
        output_bytes_cap: None,
    };
    let (_, receipt, pipes) = match start_reserved_pipe_executor(executor, request, &reservation) {
        Ok(started) => started,
        Err(error) => {
            release_executor_reservation(executor, &reservation);
            return Err(error);
        }
    };
    let child = ExecutorChild {
        executor: executor.clone(),
        receipt,
        process_id: pipes.process_id,
        ended: None,
    };
    Ok(GuestProcess::new(pipes.stdin, pipes.stdout, child).stderr(pipes.stderr))
}

/// The bridge's control of a guest's process, through its executor: it
/// ends the process only with its receipt, and releases the executor once
/// the process's end is confirmed.
struct ExecutorChild {
    executor: ExecutorManager,
    receipt: ReservedExecutorReceipt,
    process_id: u32,
    /// Its exit status, once its end was confirmed and its executor
    /// released.
    ended: Option<ExitStatus>,
}

impl ExecutorChild {
    /// Waits up to `timeout` for the process to end; once it has, its
    /// executor is disposed.
    fn wait_for(&mut self, timeout: Duration) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.ended {
            return Ok(Some(status));
        }
        let Some(status) = wait_reserved_executor(&self.executor, &self.receipt, timeout)
            .map_err(io::Error::other)?
        else {
            return Ok(None);
        };
        self.ended = Some(status);
        if let Err(error) = dispose_reserved_executor(&self.executor, &self.receipt) {
            log::warn!(
                "the plugin guest executor {} was not released: {error}",
                self.receipt.process_id()
            );
        }
        Ok(Some(status))
    }
}

impl GuestChild for ExecutorChild {
    fn id(&self) -> Option<u32> {
        Some(self.process_id)
    }

    fn kill(&mut self) -> io::Result<()> {
        if self.ended.is_some() {
            return Ok(());
        }
        terminate_reserved_executor(&self.executor, &self.receipt).map_err(io::Error::other)
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.wait_for(Duration::ZERO)
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.wait_for(WAIT_STEP)? {
                return Ok(status);
            }
        }
    }
}

impl Drop for ExecutorChild {
    /// The bridge reaps a process before it lets go of it. One it did not
    /// is ended and released here, off the caller's thread; one the
    /// executor no longer knows (the owner group's sweep took it) is gone.
    fn drop(&mut self) {
        if self.ended.is_some() || !reserved_executor_exists(&self.executor, &self.receipt) {
            return;
        }
        let (executor, receipt) = (self.executor.clone(), self.receipt.clone());
        let disposal =
            spawn_contained_thread(FaultOwner::Core, "plugin-guest-release", move || {
                if let Err(error) = dispose_reserved_executor(&executor, &receipt) {
                    log::warn!(
                        "the plugin guest executor {} was not released: {error}",
                        receipt.process_id()
                    );
                }
            });
        if let Err(error) = disposal {
            log::warn!("the plugin guest executor was not released: {error}");
        }
    }
}
