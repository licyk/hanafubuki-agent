//! The update prompt through the real binary on a pseudo-terminal, against
//! the fake release on loopback (`HANAFUBUKI_AGENT_TEST_RELEASE_URL`). The
//! binary is hard-linked out of `target/` (copied where it cannot be), where
//! it would be a build from a checkout, which is never prompted; the cache
//! is written before each launch with a fresh `checkedAtMs`, so no check
//! runs. The test is the terminal: it reads the screen with vt100 and
//! answers the cursor-position and device-attribute queries.
//!
//! Unix only: the released update installs a shell script that prints the
//! new version (as `tests/update.rs` does), and the pseudo-terminal is
//! portable-pty's Unix one. Debug builds only: a release build asks GitHub
//! whatever its environment says.
#![cfg(all(unix, debug_assertions))]

use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent::test_support::{assistant_events, configure_test_provider, FakeServer};
use hanafubuki_agent::{AgentConfig, AGENT_HOME_ENV};
use hanafubuki_agent_update::test_support::{FakeArchive, FakeAsset, FakeRelease, LATEST_PATH};
use hanafubuki_agent_update::{current_version, Version};
use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};

const BINARY: &str = env!("CARGO_BIN_EXE_hanafubuki-agent");
const DEADLINE: Duration = Duration::from_secs(120);
const ROWS: u16 = 30;
const COLUMNS: u16 = 120;
/// The interface's footer, once it is up.
const INTERFACE: &str = "fixture-model";
/// Only the prompt shows it.
const PROMPT_OPTION: &str = "Skip until next version";

/// The binary at `bin/hanafubuki-agent` in a directory of its own, its
/// home (with the fixture Provider and the cache), a fake release of
/// 99.0.0 and a fake model.
struct Installed {
    root: tempfile::TempDir,
    executable: PathBuf,
    release: FakeRelease,
    model: FakeServer,
    _model_runtime: tokio::runtime::Runtime,
}

impl Installed {
    fn new() -> Self {
        let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let executable = bin.join("hanafubuki-agent");
        if std::fs::hard_link(BINARY, &executable).is_err() {
            std::fs::copy(BINARY, &executable).unwrap();
        }
        let model_runtime = tokio::runtime::Runtime::new().unwrap();
        let answers = (0..4)
            .map(|_| (assistant_events("Done."), Duration::ZERO))
            .collect();
        let model = model_runtime.block_on(FakeServer::start(answers));
        let installed = Self {
            root,
            executable,
            release: FakeRelease::start(),
            model,
            _model_runtime: model_runtime,
        };
        let home = installed.home();
        std::fs::create_dir_all(installed.root.path().join("project/.git")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), "disabledPlugins = [\"titles\"]\n").unwrap();
        configure_test_provider(&AgentConfig::new(&home), &installed.model.base_url, None);
        installed.cache(&format!(
            r#"{{"latestVersion": "99.0.0", "checkedAtMs": {}}}"#,
            now_ms()
        ));
        installed
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn cache(&self, text: &str) {
        std::fs::write(self.home().join("update-check.json"), text).unwrap();
    }

    fn cache_text(&self) -> String {
        std::fs::read_to_string(self.home().join("update-check.json")).unwrap_or_default()
    }

    /// Publishes 99.0.0, its archive holding a script that reports it.
    fn publish(&self) {
        let latest = Version::new(99, 0, 0);
        let script = format!("#!/bin/sh\necho 'hanafubuki-agent {latest}'\n");
        let archive = FakeArchive::release(&latest, script).bytes();
        self.release.publish(
            "v99.0.0",
            &[FakeAsset::new(&FakeArchive::name(&latest), archive)],
        );
    }

    /// `program` (the installed copy unless said) started on a terminal of
    /// its own.
    fn launch(&self, program: &Path, arguments: &[&str], environment: &[(&str, &str)]) -> Term {
        let mut command = CommandBuilder::new(program);
        command.args(arguments);
        command.cwd(self.root.path().join("project"));
        command.env(AGENT_HOME_ENV, self.home());
        command.env("HANAFUBUKI_AGENT_TEST_RELEASE_URL", self.release.api_url());
        command.env("TERM", "xterm-256color");
        for variable in ["GITHUB_TOKEN", "HANAFUBUKI_AGENT_NO_UPDATE_CHECK"] {
            command.env_remove(variable);
        }
        for (key, value) in environment {
            command.env(key, value);
        }
        Term::spawn(command)
    }

    /// The data directory's lock file, which only an Agent's start makes.
    fn agent_started(&self) -> bool {
        self.home().join(".lock").exists()
    }

    fn requests(&self) -> Vec<String> {
        self.release
            .requests()
            .into_iter()
            .map(|request| request.path)
            .collect()
    }
}

/// A pseudo-terminal running one process, its screen kept by vt100.
struct Term {
    child: Box<dyn Child + Send + Sync>,
    input: Arc<Mutex<Box<dyn Write + Send>>>,
    screen: Arc<Mutex<Screen>>,
    reader: JoinHandle<()>,
}

struct Screen {
    parser: vt100::Parser,
    /// Every byte the process wrote.
    written: Vec<u8>,
}

impl Term {
    fn spawn(command: CommandBuilder) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLUMNS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let input = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let screen = Arc::new(Mutex::new(Screen {
            parser: vt100::Parser::new(ROWS, COLUMNS, 0),
            written: Vec::new(),
        }));
        let output = pair.master.try_clone_reader().unwrap();
        let reader = {
            let (input, screen) = (input.clone(), screen.clone());
            // The master stays open while the reader runs.
            let master = pair.master;
            std::thread::spawn(move || {
                let _master = master;
                answer_the_terminal(output, &input, &screen);
            })
        };
        Self {
            child,
            input,
            screen,
            reader,
        }
    }

    fn send(&self, bytes: &str) {
        let mut input = self.input.lock().unwrap();
        input.write_all(bytes.as_bytes()).unwrap();
        input.flush().unwrap();
    }

    fn contents(&self) -> String {
        self.screen.lock().unwrap().parser.screen().contents()
    }

    fn written(&self) -> String {
        String::from_utf8_lossy(&self.screen.lock().unwrap().written).into_owned()
    }

    fn wait_for(&self, text: &str) {
        let deadline = Instant::now() + DEADLINE;
        while !self.contents().contains(text) {
            assert!(
                Instant::now() < deadline,
                "waited in vain for {text:?}:\n{}",
                self.contents()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The exit code, once the process ended.
    fn exit_code(&mut self) -> u32 {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                // What the process printed last reaches the screen: the
                // reader ends at the terminal's end of output.
                while !self.reader.is_finished() {
                    assert!(Instant::now() < deadline, "the output never ended");
                    std::thread::sleep(Duration::from_millis(20));
                }
                return status.exit_code();
            }
            assert!(
                Instant::now() < deadline,
                "still running:\n{}",
                self.contents()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn is_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        if self.is_running() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Reads the process's output into the screen, answering the queries a
/// terminal answers: the cursor's position and the device attributes.
fn answer_the_terminal(
    mut output: Box<dyn Read + Send>,
    input: &Mutex<Box<dyn Write + Send>>,
    screen: &Mutex<Screen>,
) {
    let mut buffer = [0u8; 8192];
    let (mut positions, mut attributes) = (0, 0);
    // A Linux master reads EIO once the process has ended.
    while let Ok(read) = output.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let mut screen = screen.lock().unwrap();
        screen.parser.process(&buffer[..read]);
        screen.written.extend_from_slice(&buffer[..read]);
        let mut answers = String::new();
        let asked = occurrences(&screen.written, b"\x1b[6n");
        while positions < asked {
            let (row, column) = screen.parser.screen().cursor_position();
            answers.push_str(&format!("\x1b[{};{}R", row + 1, column + 1));
            positions += 1;
        }
        let asked = occurrences(&screen.written, b"\x1b[c");
        while attributes < asked {
            answers.push_str("\x1b[?62c");
            attributes += 1;
        }
        if !answers.is_empty() {
            let mut input = input.lock().unwrap();
            let _ = input.write_all(answers.as_bytes());
            let _ = input.flush();
        }
    }
}

fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// The file at `path` is the one `BINARY` is (or its copy, unchanged):
/// the updater never writes into an executable, only renames over it.
fn unchanged(path: &Path) -> (u64, u64, i64) {
    let metadata = std::fs::metadata(path).unwrap();
    (metadata.ino(), metadata.len(), metadata.mtime())
}

fn heading() -> String {
    format!("Update available · {} → 99.0.0", current_version())
}

#[test]
fn update_now_installs_the_release_and_ends_without_starting_the_agent() {
    let installed = Installed::new();
    installed.publish();
    let mut term = installed.launch(&installed.executable, &[], &[]);
    term.wait_for(&heading());
    let prompt = term.contents();
    for line in [
        "  Release notes: https://github.com/licyk/hanafubuki-agent/releases/tag/v99.0.0",
        "› 1. Update now (downloads 99.0.0 and replaces this executable)",
        "  2. Skip",
        "  3. Skip until next version",
        "  enter continue · esc skip",
    ] {
        assert!(
            prompt.lines().any(|row| row == line),
            "{line:?} in:\n{prompt}"
        );
    }

    term.send("1");
    assert_eq!(term.exit_code(), 0, "{}", term.contents());
    let screen = term.contents();
    assert!(!screen.contains(PROMPT_OPTION), "{screen}");
    assert!(
        screen.contains(&format!(
            "Updating hanafubuki-agent {} → 99.0.0 at",
            current_version()
        )),
        "{screen}"
    );
    assert!(screen.contains("Updated to 99.0.0."), "{screen}");
    let reported = std::process::Command::new(&installed.executable)
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&reported.stdout),
        "hanafubuki-agent 99.0.0\n"
    );
    assert!(!installed.agent_started());
    // The fresh lookup and the archive; no check (the cache is fresh).
    let requests = installed.requests();
    assert_eq!(requests[0], LATEST_PATH, "{requests:?}");
    assert_eq!(requests.len(), 2, "{requests:?}");
    // `update` leaves the cache: its 99.0.0 is the installed version now.
    assert!(!installed.cache_text().contains("dismissedVersion"));
}

#[test]
fn a_failed_update_ends_with_1_and_leaves_the_executable() {
    let installed = Installed::new();
    let latest = Version::new(99, 0, 0);
    let name = FakeArchive::name(&latest);
    let archive = FakeArchive::release(&latest, "unused").bytes();
    installed.release.publish(
        "v99.0.0",
        &[FakeAsset::new(&name, archive).with_digest(&format!("sha256:{}", "0".repeat(64)))],
    );
    let before = unchanged(&installed.executable);
    let mut term = installed.launch(&installed.executable, &[], &[]);
    term.wait_for(&heading());
    term.send("\r");
    assert_eq!(term.exit_code(), 1, "{}", term.contents());
    let screen = term.contents();
    let error = format!(
        "error: the download of {name} does not match its SHA-256 digest; nothing was \
         changed. Try again."
    );
    assert!(
        screen
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains(&error),
        "{screen}"
    );
    assert_eq!(unchanged(&installed.executable), before);
    assert!(!installed.agent_started());
}

/// What was typed before the prompt appeared (an Enter that would choose
/// "Update now") is dropped; Esc skips into the interface, on `resume`
/// too, and the prompt comes again at the next launch.
#[test]
fn keys_typed_early_are_dropped_and_esc_starts_the_interface() {
    let installed = Installed::new();
    installed.publish();
    let mut term = installed.launch(&installed.executable, &["resume", "--last"], &[]);
    term.send("1\r");
    term.wait_for(&heading());
    std::thread::sleep(Duration::from_millis(500));
    assert!(term.is_running());
    assert!(
        term.contents().contains(PROMPT_OPTION),
        "{}",
        term.contents()
    );

    term.send("\x1b");
    term.wait_for(INTERFACE);
    assert!(!term.contents().contains(PROMPT_OPTION));
    assert!(
        installed.requests().is_empty(),
        "{:?}",
        installed.requests()
    );
    let before = installed.cache_text();
    drop(term);

    let term = installed.launch(&installed.executable, &[], &[]);
    term.wait_for(&heading());
    assert_eq!(installed.cache_text(), before);
}

/// "Skip until next version" is written to the cache, the interface
/// starts, and the next launch goes straight to it.
#[test]
fn skip_until_next_version_is_kept_and_the_next_launch_does_not_prompt() {
    let installed = Installed::new();
    let term = installed.launch(&installed.executable, &[], &[]);
    term.wait_for(&heading());
    term.send("3");
    term.wait_for(INTERFACE);
    assert!(
        installed
            .cache_text()
            .contains(r#""dismissedVersion": "99.0.0""#),
        "{}",
        installed.cache_text()
    );
    drop(term);

    let term = installed.launch(&installed.executable, &[], &[]);
    term.wait_for(INTERFACE);
    assert!(!term.written().contains(PROMPT_OPTION));
}

/// No prompt with a prompt argument, with the check turned off by the
/// variable or the setting, or from a build in a checkout's `target/`.
#[test]
fn no_prompt_with_a_prompt_argument_the_check_off_or_a_checkout_build() {
    let installed = Installed::new();
    let executable = installed.executable.clone();
    let not_prompted = |term: Term| {
        term.wait_for(INTERFACE);
        assert!(
            !term.written().contains(PROMPT_OPTION),
            "{}",
            term.contents()
        );
    };
    not_prompted(installed.launch(&executable, &["Say done."], &[]));
    let off = [("HANAFUBUKI_AGENT_NO_UPDATE_CHECK", "1")];
    not_prompted(installed.launch(&executable, &[], &off));
    not_prompted(installed.launch(Path::new(BINARY), &[], &[]));

    let config = installed.home().join("config.toml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("\n[tui]\ncheckForUpdates = false\n");
    std::fs::write(&config, text).unwrap();
    not_prompted(installed.launch(&executable, &[], &[]));
}

/// Two instances launched at once both prompt; the second "Update now",
/// after the first's, finds the installed executable at the release and
/// says so.
#[test]
fn two_instances_prompt_and_the_second_update_finds_it_done() {
    let installed = Installed::new();
    installed.publish();
    let mut first = installed.launch(&installed.executable, &[], &[]);
    let mut second = installed.launch(&installed.executable, &[], &[]);
    first.wait_for(&heading());
    second.wait_for(&heading());

    first.send("1");
    assert_eq!(first.exit_code(), 0, "{}", first.contents());
    assert!(first.contents().contains("Updated to 99.0.0."));
    second.send("1");
    assert_eq!(second.exit_code(), 0, "{}", second.contents());
    let screen = second.contents();
    assert!(
        screen.contains("is already 99.0.0 (another process updated it)."),
        "{screen}"
    );
    assert!(!installed.agent_started());
}
