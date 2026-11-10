//! The Agent a check runs: built over temporary paths with the scripted
//! model, one session in `FullAccess` (a plugin tool's call asks no one),
//! and the plugins the check installs from directories it writes.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hanafubuki_agent::sandbox::{Sandbox, SandboxStatus};
use hanafubuki_agent::test_support::{
    assistant_events, configure_test_provider, namespace_tool_events, service_with_host, test_host,
    tool_events, tool_search_events, FakeServer, TestPaths, TEST_SHUTDOWN_DEADLINE, TEST_WAIT,
};
use hanafubuki_agent::{
    AgentApplicationService, AgentPackageInstall, AgentPackageSource, AgentPackageStatus,
    AgentPermissionMode, AgentSessionId, AgentSessionMode, AgentTurnOutcome, GuestState,
    GuestStatus, PackagePermissions, WorkspaceAccess,
};
use serde_json::{json, Value};

use crate::guest::{GUEST_ARG, TOOLS};
use crate::watchdog::step;

/// Whether this machine runs guests: under the sandbox's plugin profile,
/// checked on Linux. Says why not when it does not.
pub fn sandboxed() -> bool {
    if !cfg!(target_os = "linux") {
        eprintln!("  skipped: a sandboxed guest is checked on Linux only");
        return false;
    }
    if let SandboxStatus::Unavailable { reason } = Sandbox::detect().status() {
        eprintln!("  skipped: the sandbox is unavailable here ({reason})");
        return false;
    }
    true
}

/// The model's answer calling the tool `tool` of `namespace` (a plugin's
/// name, or `mcp__<server>`).
pub fn call(namespace: &str, tool: &str) -> Vec<u8> {
    namespace_tool_events(namespace, tool, "{}")
}

/// The model's answer calling the built-in tool `tool`.
pub fn builtin(tool: &str) -> Vec<u8> {
    tool_events(tool, "{}")
}

/// The model's `tool_search` for `query`.
pub fn search(query: &str) -> Vec<u8> {
    tool_search_events(query)
}

/// The model's final answer `text`.
pub fn answer(text: &str) -> Vec<u8> {
    assistant_events(text)
}

pub fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A Claude Code plugin `name` at `source`, with a Skill `search`.
pub fn content_plugin(source: &Path, name: &str) {
    write(
        &source.join(".claude-plugin/plugin.json"),
        &json!({ "name": name, "version": "1.0.0", "description": format!("The {name} plugin.") })
            .to_string(),
    );
    write(
        &source.join("skills/search/SKILL.md"),
        &format!(
            "---\nname: search\ndescription: Searches the {name} records.\n---\nSearch them.\n"
        ),
    );
}

/// A code plugin `name` at `source`, whose guest is this binary in `mode`:
/// every tool of [`TOOLS`] (`hang` within 1 s) and the context section
/// `<name>_board`.
pub fn code_plugin(source: &Path, name: &str, mode: &str) {
    let program = std::env::current_exe().unwrap();
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|tool| {
            let mut declared = json!({
                "name": tool,
                "description": format!("The {tool} tool."),
                "inputSchema": { "type": "object" },
            });
            if *tool == "hang" {
                declared["timeout"] = json!(1);
            }
            declared
        })
        .collect();
    write(
        &source.join(".hanafubuki-plugin/plugin.json"),
        &json!({
            "name": name,
            "version": "1.0.0",
            "runtime": {
                "type": "process",
                "command": [program.to_str().unwrap(), GUEST_ARG, name, mode],
                "interpreter": true,
            },
            "contributes": {
                "tools": tools,
                "contextSections": [{ "kind": format!("{name}_board") }],
            },
            "permissions": { "workspace": "none" },
        })
        .to_string(),
    );
}

/// What a code plugin's section says in a request.
pub fn board(name: &str) -> String {
    format!("The {name} board is open.")
}

pub struct Agent {
    pub service: AgentApplicationService,
    pub paths: TestPaths,
    pub server: FakeServer,
    pub session: AgentSessionId,
}

impl Agent {
    /// The Agent `name`, whose model answers each request with the next of
    /// `script`.
    pub async fn start(name: &str, script: Vec<Vec<u8>>) -> Self {
        step("start the Agent");
        let server = FakeServer::start(
            script
                .into_iter()
                .map(|body| (body, Duration::ZERO))
                .collect(),
        )
        .await;
        let (service, paths, _) = service_with_host(name, test_host, |_, config| {
            configure_test_provider(config, &server.base_url, None);
            // Its title requests would take the scripted answers.
            config.set_plugin_disabled("titles", true).unwrap();
        });
        service.start_plugins().await;
        let session = service
            .create_session(AgentSessionMode::Default)
            .await
            .unwrap()
            .id;
        service
            .set_session_permission_mode(session, AgentPermissionMode::FullAccess)
            .unwrap();
        Self {
            service,
            paths,
            server,
            session,
        }
    }

    /// Where the plugin `name`'s directory is written.
    pub fn source(&self, name: &str) -> PathBuf {
        self.paths.app_data_dir.join("sources").join(name)
    }

    /// Installs the plugin at `source` (as `<name>@local`), on.
    pub async fn install(&self, source: &Path) -> AgentPackageInstall {
        step(format!("install {}", source.display()));
        self.service
            .install_package(AgentPackageSource::Directory {
                path: source.to_path_buf(),
                marketplace: None,
            })
            .await
            .unwrap()
    }

    /// Writes, installs and grants the code plugin `name` in `mode`, and
    /// waits until its guest runs.
    pub async fn install_guest(&self, name: &str, mode: &str) -> GuestStatus {
        let source = self.source(name);
        code_plugin(&source, name, mode);
        let id = self.install(&source).await.package.id;
        let nothing = PackagePermissions {
            network: false,
            workspace: WorkspaceAccess::None,
            env: Vec::new(),
        };
        self.service
            .grant_package_permissions(&id, nothing)
            .await
            .unwrap();
        self.wait_guest(&id, "its start", running).await
    }

    pub async fn package(&self, id: &str) -> AgentPackageStatus {
        self.service
            .packages()
            .await
            .unwrap()
            .into_iter()
            .find(|package| package.id == id)
            .unwrap_or_else(|| panic!("no package {id}"))
    }

    pub async fn guest(&self, id: &str) -> GuestStatus {
        self.package(id).await.guest.unwrap().status
    }

    /// The guest of `id` once `done` holds for it.
    pub async fn wait_guest(
        &self,
        id: &str,
        what: &str,
        done: impl Fn(&GuestStatus) -> bool,
    ) -> GuestStatus {
        step(format!("wait for {what} of {id}"));
        let deadline = Instant::now() + TEST_WAIT;
        loop {
            let status = self.guest(id).await;
            if done(&status) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "{id} never reached {what}: {status:#?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The guest of `id` once it runs a process other than `before`'s.
    pub async fn restarted(&self, id: &str, before: &GuestStatus) -> GuestStatus {
        let previous = before.process_id;
        self.wait_guest(id, "its restart", |status| {
            running(status) && status.process_id != previous
        })
        .await
    }

    /// Runs a turn on `text` and returns how it ended.
    pub async fn turn(&self, text: &str) -> AgentTurnOutcome {
        step(format!("turn {text:?}"));
        tokio::time::timeout(
            TEST_WAIT,
            self.service
                .start_turn(self.session, text.to_string().into()),
        )
        .await
        .unwrap_or_else(|_| panic!("the turn {text:?} did not end"))
        .unwrap()
    }

    /// The model's request `index`, from the first.
    pub fn request(&self, index: usize) -> String {
        let requests = self.server.requests();
        let request = requests
            .get(index)
            .unwrap_or_else(|| panic!("only {} requests", requests.len()));
        String::from_utf8(request.body.clone()).unwrap()
    }

    /// The model's latest request.
    pub fn last_request(&self) -> String {
        self.request(self.server.requests().len() - 1)
    }

    /// Runs a turn whose model calls `<plugin>__echo` and then answers, as
    /// the script has it, and checks the call ran and the turn ended
    /// normally: the session is still usable.
    pub async fn echo_turn(&self, plugin: &str, final_answer: &str) {
        let outcome = self.turn(&format!("Echo through {plugin}.")).await;
        assert_eq!(outcome.final_answer, final_answer);
        let after = self.last_request();
        assert!(
            after.contains(&format!(r#"\"by\":\"{plugin}\""#)),
            "{plugin}__echo did not run: {after}"
        );
    }

    pub async fn finish(self) {
        step("shut down");
        assert!(self
            .service
            .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
            .is_empty());
        let _ = std::fs::remove_dir_all(&self.paths.app_data_dir);
    }
}

pub fn running(status: &GuestStatus) -> bool {
    status.state == GuestState::Running && status.process_id.is_some()
}

/// Whether the process `pid` is gone.
#[cfg(target_os = "linux")]
pub fn gone(pid: u32) -> bool {
    !Path::new(&format!("/proc/{pid}")).exists()
}

/// Kills the process `pid` with `SIGKILL`, as `kill -9` (or the system,
/// out of memory) does.
#[cfg(unix)]
pub fn kill_hard(pid: u32) {
    // SAFETY: sending a signal to a process id.
    let killed = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    assert_eq!(killed, 0, "{}", std::io::Error::last_os_error());
}
