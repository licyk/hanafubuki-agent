//! Windows PowerShell 5.1 (`powershell.exe`, never `pwsh`) and `cmd.exe` run
//! as `exec_command` runs them, through the real executor in pipe and PTY
//! mode: the garbling of the plan's §4.9, (a) to (g), and output lost in the
//! middle of a long listing.
//!
//! The runner's code pages are 1252 (ANSI) and 437 (OEM), so text a native
//! program writes in the ANSI code page is encoded with
//! `[Text.Encoding]::Default`, never spelled as GBK, and `cmd.exe`'s console
//! code page shows with `é` (0x82 in code page 437); the code page 936 cases
//! are the decoder's unit tests in `hanafubuki-agent-process`.

use super::*;

use std::time::{Duration, Instant};

use hanafubuki_agent_approval::PermissionActionDetails;
use hanafubuki_agent_execution::{
    AgentExecutionId, ExecCommandRequest, PlatformShell, WriteStdinRequest,
};
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::SandboxSettings;

const YIELD_MS: u64 = 30_000;
/// How long a command may take in all, over several polls.
const DEADLINE: Duration = Duration::from_secs(240);

/// Text the runner's ANSI code page (1252) can encode.
const ANSI_TEXT: &str = "café ü";

/// The script as `exec_command` runs it. Under PowerShell, with PowerShell 7
/// off the PATH, the shell resolves to Windows PowerShell 5.1.
fn request(shell: PlatformShell, script: &str, tty: bool) -> ExecCommandRequest {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let environment = match shell {
        PlatformShell::PowerShell => BTreeMap::from([(
            "PATH".to_string(),
            format!(r"{root}\System32\WindowsPowerShell\v1.0;{root}\System32;{root}"),
        )]),
        _ => BTreeMap::new(),
    };
    ExecCommandRequest {
        cmd: script.to_string(),
        shell: Some(shell),
        login: false,
        tty,
        yield_time_ms: YIELD_MS,
        max_output_bytes: None,
        workdir: Some(std::env::current_dir().unwrap()),
        environment,
        origin: ApprovalOrigin::Model,
        sandbox: SandboxSettings::full_access(),
        sandbox_escalation: None,
    }
}

struct Ran {
    output: String,
    truncated: bool,
}

/// Runs `request` to its end, polling as the model does, and returns all it
/// printed.
fn run_request(request: ExecCommandRequest) -> Ran {
    let started = Instant::now();
    let max_output_bytes = request.max_output_bytes;
    let (service, first) = start(request);
    finish(&service, first, None, max_output_bytes, started)
}

/// Polls a session from `first` on, writing `chars` first, until it ends.
fn finish(
    service: &AgentExecutionService,
    first: UnifiedExecResult,
    mut chars: Option<&str>,
    max_output_bytes: Option<usize>,
    started: Instant,
) -> Ran {
    let mut output = first.output;
    let mut truncated = first.truncated;
    let mut session_id = first.session_id;
    while let Some(id) = session_id {
        assert!(started.elapsed() < DEADLINE, "still running: {output:?}");
        let polled = service
            .write_stdin(
                &scope(),
                WriteStdinRequest {
                    session_id: AgentExecutionId::from_untrusted(id).unwrap(),
                    chars: chars.take().unwrap_or_default().to_string(),
                    yield_time_ms: YIELD_MS,
                    max_output_bytes,
                },
                &CancellationToken::default(),
            )
            .unwrap();
        output.push_str(&polled.output);
        truncated |= polled.truncated;
        session_id = polled.session_id;
    }
    Ran { output, truncated }
}

fn run(script: &str, tty: bool) -> String {
    run_request(request(PlatformShell::PowerShell, script, tty)).output
}

fn start(request: ExecCommandRequest) -> (AgentExecutionService, UnifiedExecResult) {
    let service = AgentExecutionService::new(ExecutorManager::default());
    let scope = scope();
    let action = service.prepare_exec_command_action(&request).unwrap();
    if request.shell == Some(PlatformShell::PowerShell) {
        let Some(PermissionActionDetails::Command { program, .. }) = action.details() else {
            panic!("a shell action names its program");
        };
        assert!(
            program
                .to_ascii_lowercase()
                .ends_with(r"\windowspowershell\v1.0\powershell.exe"),
            "{program}"
        );
    }
    let yield_ms = request.yield_time_ms;
    let max_output_bytes = request.max_output_bytes;
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let result = service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            yield_ms,
            max_output_bytes,
            &CancellationToken::default(),
        )
        .unwrap();
    (service, result)
}

fn scope() -> AgentExecutionScope {
    AgentExecutionScope::for_agent_session(51).unwrap()
}

/// Answers a prompt with non-ASCII text through `write_stdin`, as the model
/// does in a PTY session.
fn answer(script: &str) -> String {
    let started = Instant::now();
    let mut request = request(PlatformShell::PowerShell, script, true);
    request.yield_time_ms = 5_000;
    let (service, yielded) = start(request);
    assert!(
        yielded.session_id.is_some(),
        "finished before its input: {:?}",
        yielded.output
    );
    finish(&service, yielded, Some("中文 ü\r"), None, started).output
}

/// Writes `text` to standard output in the ANSI code page, through the raw
/// handle, as a native program would.
fn ansi_write(text: &str) -> String {
    format!(
        "$bytes = [Text.Encoding]::Default.GetBytes(\"{text}`n\"); \
         $out = [Console]::OpenStandardOutput(); \
         $out.Write($bytes, 0, $bytes.Length); $out.Flush()"
    )
}

fn assert_contains(output: &str, expected: &str) {
    assert!(output.contains(expected), "{expected:?} in {output:?}");
}

/// Every numbered line arrived whole, or the output says what it left out.
fn assert_every_line_or_a_marker(ran: &Ran, lines: usize) {
    let missing = (1..=lines)
        .filter(|number| {
            let line = format!("line {number} of the listing");
            !ran.output.lines().any(|shown| shown.trim_end() == line)
        })
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty() || (ran.truncated && ran.output.contains("omitted")),
        "missing {missing:?} (truncated: {}) in {:?}",
        ran.truncated,
        ran.output
    );
}

#[test]
fn the_shell_resolved_is_windows_powershell_5_1() {
    let output = run("$PSVersionTable.PSVersion.Major", false);
    assert_eq!(output.trim(), "5", "{output:?}");
}

/// `chcp` shows the console's input code page, which the prefix leaves as
/// it is (the output code page it sets: `cmd /c echo` below).
fn the_prefix_sets_the_console_input_code_page_to_utf8(tty: bool) {
    assert_contains(&run("chcp.com", tty), "65001");
}

#[test]
fn pipe_the_prefix_sets_the_console_input_code_page_to_utf8() {
    the_prefix_sets_the_console_input_code_page_to_utf8(false);
}

#[test]
fn pty_the_prefix_sets_the_console_input_code_page_to_utf8() {
    the_prefix_sets_the_console_input_code_page_to_utf8(true);
}

fn write_output_keeps_cjk_and_accents(tty: bool) {
    assert_contains(&run("Write-Output '中文 ü'", tty), "中文 ü");
}

#[test]
fn pipe_write_output_keeps_cjk_and_accents() {
    write_output_keeps_cjk_and_accents(false);
}

#[test]
fn pty_write_output_keeps_cjk_and_accents() {
    write_output_keeps_cjk_and_accents(true);
}

/// (a): a native program writing the ANSI code page.
fn ansi_code_page_output_of_a_native_program_is_decoded(tty: bool) {
    assert_contains(&run(&ansi_write(ANSI_TEXT), tty), ANSI_TEXT);
}

#[test]
fn pipe_ansi_code_page_output_of_a_native_program_is_decoded() {
    ansi_code_page_output_of_a_native_program_is_decoded(false);
}

#[test]
fn pty_ansi_code_page_output_of_a_native_program_is_decoded() {
    ansi_code_page_output_of_a_native_program_is_decoded(true);
}

/// (a): PowerShell's UTF-8 and a native program's ANSI code page in one
/// stream.
fn utf8_and_ansi_code_page_output_mix_in_one_stream(tty: bool) {
    let output = run(
        &format!(
            "[Console]::Out.WriteLine('中文 ü'); {}",
            ansi_write(ANSI_TEXT)
        ),
        tty,
    );
    assert_contains(&output, "中文 ü");
    assert_contains(&output, ANSI_TEXT);
}

#[test]
fn pipe_utf8_and_ansi_code_page_output_mix_in_one_stream() {
    utf8_and_ansi_code_page_output_mix_in_one_stream(false);
}

#[test]
fn pty_utf8_and_ansi_code_page_output_mix_in_one_stream() {
    utf8_and_ansi_code_page_output_mix_in_one_stream(true);
}

/// (a): a program that follows the console's output code page.
fn cmd_echo_keeps_cjk(tty: bool) {
    assert_contains(&run("cmd /c echo 中文", tty), "中文");
}

#[test]
fn pipe_cmd_echo_keeps_cjk() {
    cmd_echo_keeps_cjk(false);
}

#[test]
fn pty_cmd_echo_keeps_cjk() {
    cmd_echo_keeps_cjk(true);
}

/// (b): PowerShell decodes a native program's captured output itself.
fn captured_cmd_echo_keeps_cjk(tty: bool) {
    assert_contains(&run("$x = cmd /c echo 中文; $x", tty), "中文");
}

#[test]
fn pipe_captured_cmd_echo_keeps_cjk() {
    captured_cmd_echo_keeps_cjk(false);
}

#[test]
fn pty_captured_cmd_echo_keeps_cjk() {
    captured_cmd_echo_keeps_cjk(true);
}

/// (c): input piped to a native program is encoded with `$OutputEncoding`.
fn input_piped_to_a_native_program_keeps_cjk(tty: bool) {
    assert_contains(&run("'中文' | findstr .", tty), "中文");
}

#[test]
fn pipe_input_piped_to_a_native_program_keeps_cjk() {
    input_piped_to_a_native_program_keeps_cjk(false);
}

#[test]
fn pty_input_piped_to_a_native_program_keeps_cjk() {
    input_piped_to_a_native_program_keeps_cjk(true);
}

/// (e) and (g): a progress bar leaves nothing behind, as text or as CLIXML.
fn progress_leaves_no_trace(tty: bool) {
    let output = run(
        "for ($i = 0; $i -le 100; $i += 25) { \
           Write-Progress -Activity 'Copying' -Status \"$i%\" -PercentComplete $i; \
           Start-Sleep -Milliseconds 100 }; \
         Write-Progress -Activity 'Copying' -Completed; 'done'",
        tty,
    );
    assert_contains(&output, "done");
    assert!(!output.contains("Copying"), "{output:?}");
    assert!(!output.contains("CLIXML"), "{output:?}");
}

#[test]
fn pipe_progress_leaves_no_trace() {
    progress_leaves_no_trace(false);
}

#[test]
fn pty_progress_leaves_no_trace() {
    progress_leaves_no_trace(true);
}

/// (g): a failing cmdlet's error is text, not serialized `#< CLIXML`.
fn a_failing_cmdlet_reports_plain_text(tty: bool) {
    let output = run("Get-Item 'C:\\hanafubuki-does-not-exist'; 'after'", tty);
    assert_contains(&output, "hanafubuki-does-not-exist");
    assert_contains(&output, "after");
    assert!(!output.contains("CLIXML"), "{output:?}");
}

#[test]
fn pipe_a_failing_cmdlet_reports_plain_text() {
    a_failing_cmdlet_reports_plain_text(false);
}

#[test]
fn pty_a_failing_cmdlet_reports_plain_text() {
    a_failing_cmdlet_reports_plain_text(true);
}

/// (f): no byte-order mark before the first output.
fn output_starts_without_a_byte_order_mark(tty: bool) {
    let output = run("'first'", tty);
    assert_contains(&output, "first");
    assert!(!output.contains('\u{feff}'), "{output:?}");
}

#[test]
fn pipe_output_starts_without_a_byte_order_mark() {
    output_starts_without_a_byte_order_mark(false);
}

#[test]
fn pty_output_starts_without_a_byte_order_mark() {
    output_starts_without_a_byte_order_mark(true);
}

/// (d): `write_stdin` reaches the program as the text the model wrote.
#[test]
fn pty_console_input_receives_non_ascii_text() {
    assert_contains(
        &answer("$line = [Console]::ReadLine(); \"got:$line\""),
        "got:中文 ü",
    );
}

#[test]
fn pty_read_host_receives_non_ascii_text() {
    assert_contains(&answer("$line = Read-Host; \"got:$line\""), "got:中文 ü");
}

/// (a), as reported: the Cmd shell's built-ins write the console's code page.
fn cmd_built_ins_write_text_the_decoder_reads(tty: bool) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("café.txt"), b"x").unwrap();
    let mut request = request(PlatformShell::Cmd, "echo café & dir /b", tty);
    request.workdir = Some(directory.path().to_path_buf());
    let output = run_request(request).output;
    assert!(!output.contains('\u{fffd}'), "{output:?}");
    assert_eq!(output.matches("café").count(), 2, "{output:?}");
}

#[test]
fn pipe_cmd_built_ins_write_text_the_decoder_reads() {
    cmd_built_ins_write_text_the_decoder_reads(false);
}

#[test]
fn pty_cmd_built_ins_write_text_the_decoder_reads() {
    cmd_built_ins_write_text_the_decoder_reads(true);
}

/// As reported: a long `dir` lost its middle without a marker.
fn a_long_listing_arrives_whole(shell: PlatformShell, tty: bool) {
    let script = match shell {
        PlatformShell::Cmd => "for /L %i in (1,1,300) do @echo line %i of the listing",
        _ => "foreach ($i in 1..300) { \"line $i of the listing\" }",
    };
    assert_every_line_or_a_marker(&run_request(request(shell, script, tty)), 300);
}

#[test]
fn pipe_a_long_cmd_listing_arrives_whole() {
    a_long_listing_arrives_whole(PlatformShell::Cmd, false);
}

#[test]
fn pty_a_long_cmd_listing_arrives_whole() {
    a_long_listing_arrives_whole(PlatformShell::Cmd, true);
}

#[test]
fn pipe_a_long_powershell_listing_arrives_whole() {
    a_long_listing_arrives_whole(PlatformShell::PowerShell, false);
}

#[test]
fn pty_a_long_powershell_listing_arrives_whole() {
    a_long_listing_arrives_whole(PlatformShell::PowerShell, true);
}

/// A listing longer than the model's `max_output_bytes` says so.
fn a_listing_over_the_output_bound_says_what_it_left_out(tty: bool) {
    let mut request = request(
        PlatformShell::Cmd,
        "for /L %i in (1,1,300) do @echo line %i of the listing",
        tty,
    );
    request.max_output_bytes = Some(4096);
    assert_every_line_or_a_marker(&run_request(request), 300);
}

#[test]
fn pipe_a_listing_over_the_output_bound_says_what_it_left_out() {
    a_listing_over_the_output_bound_says_what_it_left_out(false);
}

#[test]
fn pty_a_listing_over_the_output_bound_says_what_it_left_out() {
    a_listing_over_the_output_bound_says_what_it_left_out(true);
}

/// As reported from `dir`: the head arrives before the first poll, the rest
/// after it, more than `max_output_bytes`, which also bounds the buffer.
#[test]
fn pipe_a_slow_listing_over_the_output_bound_says_what_it_left_out() {
    let mut request = request(
        PlatformShell::Cmd,
        "echo line 1 of the listing& ping -n 4 127.0.0.1 >NUL & \
         for /L %i in (2,1,300) do @echo line %i of the listing",
        false,
    );
    request.yield_time_ms = 1_000;
    request.max_output_bytes = Some(4096);
    assert_every_line_or_a_marker(&run_request(request), 300);
}
