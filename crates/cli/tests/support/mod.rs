//! A temporary Agent home and a working directory in a Git repository, and
//! the binary run in them.

// Each suite uses part of the fixtures.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use hanafubuki_agent::test_support::configure_test_provider;
use hanafubuki_agent::{AgentConfig, AGENT_HOME_ENV};

pub struct Home {
    _root: tempfile::TempDir,
    pub home: PathBuf,
    /// The Git repository the working directory is in.
    pub project: PathBuf,
    pub cwd: PathBuf,
}

impl Home {
    /// A home whose `config.toml` turns the `titles` plugin off, so a turn
    /// makes only its own model requests.
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = root.path().join("project");
        let cwd = project.join("src");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), "disabledPlugins = [\"titles\"]\n").unwrap();
        Self {
            _root: root,
            home,
            project,
            cwd,
        }
    }

    /// The fixture Provider at `base_url`, the default, with its model.
    pub fn with_provider(self, base_url: &str) -> Self {
        configure_test_provider(&AgentConfig::new(&self.home), base_url, None);
        self
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_hanafubuki-agent"));
        command
            .env(AGENT_HOME_ENV, &self.home)
            .current_dir(&self.cwd)
            .stdin(Stdio::null());
        command
    }

    pub fn run(&self, arguments: &[&str]) -> Run {
        Run(self.command().args(arguments).output().unwrap())
    }

    pub fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.home.join(name)).unwrap_or_default()
    }
}

pub struct Run(pub Output);

impl Run {
    pub fn code(&self) -> i32 {
        self.0.status.code().unwrap()
    }

    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.0.stdout).into_owned()
    }

    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.0.stderr).into_owned()
    }

    /// Asserts the exit code, showing both outputs when it is another.
    pub fn expect(self, code: i32) -> Self {
        assert_eq!(
            self.code(),
            code,
            "stdout:\n{}\nstderr:\n{}",
            self.stdout(),
            self.stderr()
        );
        self
    }
}
