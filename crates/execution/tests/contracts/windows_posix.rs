//! The POSIX shell on Windows, the bash of Git for Windows and of MSYS2,
//! run as `exec_command` runs it, through the real executor in pipe and
//! PTY mode. A flavour that is not installed is skipped, and says so on
//! standard error, which the test harness does not capture.

use super::*;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use hanafubuki_agent_approval::PermissionActionDetails;
use hanafubuki_agent_execution::{
    AgentExecutionId, ExecCommandRequest, PlatformShell, WriteStdinRequest,
};
use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::SandboxSettings;

const YIELD_MS: u64 = 30_000;
/// How long a command may take in all, over several polls: a login shell
/// of MSYS2 starts slowly on a CI runner.
const DEADLINE: Duration = Duration::from_secs(240);
/// The longest script a POSIX shell on Windows takes, in UTF-16 units.
const MAX_SCRIPT_UNITS: usize = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flavour {
    Git,
    Msys2,
}

impl Flavour {
    fn root(self) -> PathBuf {
        match self {
            Self::Git => PathBuf::from(variable("ProgramFiles")).join("Git"),
            Self::Msys2 => PathBuf::from(format!(r"{}\msys64", variable("SystemDrive"))),
        }
    }

    fn shell(self) -> PathBuf {
        self.root().join(r"usr\bin\bash.exe")
    }

    /// The request's variables that make the search find this flavour,
    /// beside `cargo`'s directory as a Windows program's: Git for Windows
    /// through its launchers' directory (`bin`, which holds no MSYS
    /// program) first on the PATH; MSYS2 through the installations looked
    /// at last, with only WSL's launcher on the PATH (in `System32`) and
    /// Git for Windows' installation out of sight.
    fn environment(self) -> BTreeMap<String, String> {
        let root = variable("SystemRoot");
        let windows = format!(r"{};{root}\System32;{root}", cargo_directory().display());
        match self {
            Self::Git => BTreeMap::from([(
                "PATH".to_string(),
                format!(r"{}\bin;{windows}", self.root().display()),
            )]),
            Self::Msys2 => {
                let nowhere = std::env::temp_dir()
                    .join("hanafubuki-agent-no-git")
                    .display()
                    .to_string();
                BTreeMap::from([
                    ("PATH".to_string(), windows),
                    ("ProgramFiles".to_string(), nowhere.clone()),
                    ("ProgramFiles(x86)".to_string(), nowhere.clone()),
                    ("LOCALAPPDATA".to_string(), nowhere),
                ])
            }
        }
    }

    /// Whether this flavour can be tested here; when not, says why.
    fn available(self) -> bool {
        let skipped = if std::env::var_os("MSYSTEM").is_some() {
            Some(
                "the tests run in an MSYS2 or Git Bash shell, whose own bash is found first"
                    .to_string(),
            )
        } else if !self.shell().is_file() {
            Some(format!("{} is not installed", self.shell().display()))
        } else {
            None
        };
        if let Some(reason) = &skipped {
            report(&format!("{self:?} skipped: {reason}"));
        }
        skipped.is_none()
    }

    /// Runs one login shell before the first check: MSYS2's first one
    /// copies its skeleton files and says so, at length.
    fn warm_up(self) {
        static WARM: [OnceLock<()>; 2] = [OnceLock::new(), OnceLock::new()];
        WARM[self as usize].get_or_init(|| {
            let ran = run_request(self, request(self, "true", true, false));
            report(&format!(
                "{self:?} runs {} (first login shell: exit {:?})",
                self.shell().display(),
                ran.exit_code
            ));
        });
    }
}

/// Writes past the harness's capture, so a CI log shows what ran.
fn report(line: &str) {
    let _ = writeln!(std::io::stderr(), "windows_posix: {line}");
}

fn variable(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is set on Windows"))
}

fn cargo_directory() -> PathBuf {
    Path::new(&variable("CARGO"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// `check` on every flavour installed here, in pipe and in PTY mode.
fn on_each_flavour(check: impl Fn(Flavour, bool)) {
    for flavour in [Flavour::Git, Flavour::Msys2] {
        if flavour.available() {
            flavour.warm_up();
            for tty in [false, true] {
                check(flavour, tty);
            }
        }
    }
}

fn request(flavour: Flavour, script: &str, login: bool, tty: bool) -> ExecCommandRequest {
    ExecCommandRequest {
        cmd: script.to_string(),
        shell: Some(PlatformShell::Posix),
        login,
        tty,
        yield_time_ms: YIELD_MS,
        max_output_bytes: None,
        timeout_ms: 600_000,
        workdir: Some(std::env::current_dir().unwrap()),
        environment: flavour.environment(),
        origin: ApprovalOrigin::Model,
        sandbox: SandboxSettings::full_access(),
        sandbox_escalation: None,
    }
}

struct Ran {
    output: String,
    exit_code: Option<i32>,
}

impl Ran {
    /// The lines that start with `marker`, without it.
    fn marked(&self, marker: &str) -> Vec<&str> {
        self.output
            .lines()
            .filter_map(|line| line.trim_end().strip_prefix(marker))
            .collect()
    }
}

fn scope() -> AgentExecutionScope {
    AgentExecutionScope::for_agent_session(61).unwrap()
}

/// Starts `request` as the model's tool call does, checking that it runs
/// the flavour's own shell.
fn start(
    flavour: Flavour,
    request: ExecCommandRequest,
) -> (AgentExecutionService, UnifiedExecResult) {
    let service = AgentExecutionService::new(ExecutorManager::default());
    let action = service.prepare_exec_command_action(&request).unwrap();
    let Some(PermissionActionDetails::Command { program, .. }) = action.details() else {
        panic!("a shell action names its program");
    };
    assert_eq!(
        ResolvedPath::resolve(program).unwrap(),
        ResolvedPath::resolve(flavour.shell()).unwrap(),
        "{flavour:?} runs {program}"
    );
    let yield_ms = request.yield_time_ms;
    let started = service
        .start_exec_command(
            scope(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let first = service
        .wait_unified_execution(
            &scope(),
            started.execution_id,
            yield_ms,
            None,
            &CancellationToken::default(),
        )
        .unwrap();
    (service, first)
}

fn poll(service: &AgentExecutionService, id: u64, chars: &str) -> UnifiedExecResult {
    service
        .write_stdin(
            &scope(),
            WriteStdinRequest {
                session_id: AgentExecutionId::from_untrusted(id).unwrap(),
                chars: chars.to_string(),
                yield_time_ms: YIELD_MS,
                max_output_bytes: None,
            },
            &CancellationToken::default(),
        )
        .unwrap()
}

/// Runs `request` to its end, polling as the model does.
fn run_request(flavour: Flavour, request: ExecCommandRequest) -> Ran {
    let started = Instant::now();
    let (service, mut result) = start(flavour, request);
    let mut output = std::mem::take(&mut result.output);
    while let Some(id) = result.session_id {
        assert!(started.elapsed() < DEADLINE, "still running: {output:?}");
        result = poll(&service, id, "");
        output.push_str(&result.output);
    }
    Ran {
        output,
        exit_code: result.exit_code,
    }
}

fn run(flavour: Flavour, script: &str, login: bool, tty: bool) -> Ran {
    run_request(flavour, request(flavour, script, login, tty))
}

fn same_place(left: &str, right: &Path) -> bool {
    ResolvedPath::resolve(left).ok() == ResolvedPath::resolve(right).ok()
}

/// The command starts in the requested directory, a login shell too
/// (MSYS2's `/etc/profile` changes to `$HOME` without `CHERE_INVOKING`).
fn runs_in_the_workdir(flavour: Flavour, tty: bool) {
    let workdir = tempfile::Builder::new()
        .prefix("work dir ")
        .tempdir()
        .unwrap();
    for login in [false, true] {
        let mut request = request(flavour, "printf 'pwd=%s\\n' \"$(pwd -W)\"", login, tty);
        request.workdir = Some(workdir.path().to_path_buf());
        let ran = run_request(flavour, request);
        let printed = ran.marked("pwd=");
        assert!(
            printed.len() == 1 && same_place(printed[0], workdir.path()),
            "{flavour:?} login={login} tty={tty}: {:?} in {}",
            ran.output,
            workdir.path().display()
        );
    }
}

#[test]
fn pipe_and_pty_a_command_runs_in_its_workdir() {
    on_each_flavour(runs_in_the_workdir);
}

/// The MSYS programs and the Windows PATH's both run, in a login shell
/// (which builds its own PATH) and in one that is not (which keeps the
/// PATH it was given).
fn finds_msys_and_windows_programs(flavour: Flavour, tty: bool) {
    for login in [false, true] {
        let ran = run(
            flavour,
            "printf 'uname=%s\\n' \"$(uname -o)\"; ls / >/dev/null && echo ok=ls; \
             cargo --version >/dev/null && echo ok=cargo",
            login,
            tty,
        );
        assert_eq!(
            ran.marked("uname="),
            ["Msys"],
            "{flavour:?} login={login} tty={tty}: {:?}",
            ran.output
        );
        assert_eq!(
            ran.marked("ok="),
            ["ls", "cargo"],
            "{flavour:?} login={login} tty={tty}: {:?}",
            ran.output
        );
        assert_eq!(ran.exit_code, Some(0));
    }
}

#[test]
fn pipe_and_pty_msys_and_windows_programs_are_found() {
    on_each_flavour(finds_msys_and_windows_programs);
}

/// The script reaches bash as it was written: quotes, globs, backslashes,
/// variables, newlines and non-ASCII text, in a script with spaces and in
/// one without.
fn the_script_arrives_exactly(flavour: Flavour, tty: bool) {
    let ran = run(
        flavour,
        "v='it'\\''s $HOME'; printf '<%s>\\n' \"a\\\"b\" '*' 'c\\\\d' \"$v\" '中文 é' \"{x,y}\" '~'\n\
         printf '<%s>\\n' end\\\\",
        false,
        tty,
    );
    assert_eq!(
        ran.marked("<"),
        [
            "a\"b>",
            "*>",
            "c\\\\d>",
            "it's $HOME>",
            "中文 é>",
            "{x,y}>",
            "~>",
            "end\\>"
        ],
        "{flavour:?} tty={tty}: {:?}",
        ran.output
    );
    let ran = run(
        flavour,
        "printf${IFS}'<%s>\\n'${IFS}'a*b'\necho${IFS}\"<q\\\"d>\"",
        false,
        tty,
    );
    assert_eq!(
        ran.marked("<"),
        ["a*b>", "q\"d>"],
        "{flavour:?} tty={tty}: {:?}",
        ran.output
    );
}

#[test]
fn pipe_and_pty_a_script_arrives_exactly() {
    on_each_flavour(the_script_arrives_exactly);
}

/// The longest script runs whole; a longer one is refused before it is
/// approved, since the MSYS runtime would cut it short.
fn the_longest_script_runs_whole(flavour: Flavour, tty: bool) {
    let (head, tail) = ("x='", "'; echo \"length=${#x}\"");
    let body = "ab\\\\ 中\"*"
        .chars()
        .cycle()
        .take(MAX_SCRIPT_UNITS - head.len() - tail.len())
        .collect::<String>();
    let script = format!("{head}{body}{tail}");
    assert_eq!(script.encode_utf16().count(), MAX_SCRIPT_UNITS);
    let ran = run(flavour, &script, false, tty);
    assert_eq!(
        ran.marked("length="),
        [body.chars().count().to_string()],
        "{flavour:?} tty={tty}: {:?}",
        ran.output
    );

    let longer = request(flavour, &format!("{script} "), false, tty);
    let refused = AgentExecutionService::new(ExecutorManager::default())
        .prepare_exec_command_action(&longer)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("\"power_shell\""), "{refused}");
}

#[test]
fn pipe_and_pty_the_longest_script_runs_whole() {
    on_each_flavour(the_longest_script_runs_whole);
}

fn exit_codes_propagate(flavour: Flavour, tty: bool) {
    for login in [false, true] {
        assert_eq!(
            run(flavour, "exit 7", login, tty).exit_code,
            Some(7),
            "{flavour:?} login={login} tty={tty}"
        );
    }
}

#[test]
fn pipe_and_pty_an_exit_code_propagates() {
    on_each_flavour(exit_codes_propagate);
}

/// Whether the Windows process `pid` still runs.
fn running(pid: &str) -> bool {
    let listed = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout).contains(&format!("\"{pid}\""))
}

/// A command interrupted with Ctrl+C, then stopped, leaves neither its
/// shell nor the program it started running.
fn a_stop_leaves_no_process(flavour: Flavour, tty: bool) {
    let mut request = request(
        flavour,
        "sleep 60 & echo \"pids=$(cat /proc/$$/winpid) $(cat /proc/$!/winpid)\"; wait",
        false,
        tty,
    );
    request.yield_time_ms = 5_000;
    let started = Instant::now();
    let (service, mut result) = start(flavour, request);
    let mut output = std::mem::take(&mut result.output);
    while !output.contains("pids=") && started.elapsed() < DEADLINE {
        result = poll(&service, result.session_id.expect("still running"), "");
        output.push_str(&result.output);
    }
    let pids = output
        .lines()
        .find_map(|line| line.trim_end().strip_prefix("pids="))
        .unwrap_or_else(|| panic!("{output:?}"))
        .split(' ')
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(pids.len(), 2, "{output:?}");
    assert!(pids.iter().all(|pid| running(pid)), "{pids:?}");

    let id = result.session_id.expect("sleeping");
    let interrupted = poll(&service, id, "\u{3}");
    report(&format!(
        "{flavour:?} tty={tty}: after Ctrl+C the session {}",
        if interrupted.session_id.is_none() {
            "ended"
        } else {
            "still ran"
        }
    ));
    let stopped = service.stop_background_terminals_confirmed(&scope());
    assert!(stopped.retained.is_empty(), "{:?}", stopped.retained);
    let gone = Instant::now();
    while pids.iter().any(|pid| running(pid)) {
        assert!(
            gone.elapsed() < Duration::from_secs(20),
            "{flavour:?} tty={tty}: {pids:?} still run"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn pipe_and_pty_a_stop_leaves_no_process() {
    on_each_flavour(a_stop_leaves_no_process);
}
