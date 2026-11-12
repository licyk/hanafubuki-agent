//! Windows PowerShell 5.1 (`powershell.exe`, never `pwsh`) run as
//! `exec_command` runs it, through the real executor in pipe and PTY mode:
//! the garbling of the plan's §4.9, (a) to (g).
//!
//! The runner's code pages are 1252 (ANSI) and 437 (OEM), so text a native
//! program writes in the ANSI code page is encoded with
//! `[Text.Encoding]::Default`, never spelled as GBK; the code page 936 cases
//! are the decoder's unit tests in `hanafubuki-agent-process`.

use super::*;

use hanafubuki_agent_approval::PermissionActionDetails;
use hanafubuki_agent_execution::{
    AgentExecutionId, ExecCommandRequest, PlatformShell, WriteStdinRequest,
};
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::SandboxSettings;

const YIELD_MS: u64 = 30_000;

/// Text the runner's ANSI code page (1252) can encode.
const ANSI_TEXT: &str = "café ü";

/// The script as `exec_command` runs it under Windows PowerShell: with
/// PowerShell 7 off the PATH, the shell resolves to 5.1.
fn request(script: &str, tty: bool) -> ExecCommandRequest {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    ExecCommandRequest {
        cmd: script.to_string(),
        shell: Some(PlatformShell::PowerShell),
        login: false,
        tty,
        yield_time_ms: YIELD_MS,
        max_output_bytes: Some(64 * 1024),
        workdir: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::from([(
            "PATH".to_string(),
            format!(r"{root}\System32\WindowsPowerShell\v1.0;{root}\System32;{root}"),
        )]),
        origin: ApprovalOrigin::Model,
        sandbox: SandboxSettings::full_access(),
        sandbox_escalation: None,
    }
}

fn start(script: &str, tty: bool, yield_ms: u64) -> (AgentExecutionService, UnifiedExecResult) {
    let service = AgentExecutionService::new(ExecutorManager::default());
    let scope = scope();
    let mut request = request(script, tty);
    request.yield_time_ms = yield_ms;
    let action = service.prepare_exec_command_action(&request).unwrap();
    let Some(PermissionActionDetails::Command { program, .. }) = action.details() else {
        panic!("a shell action names its program");
    };
    assert!(
        program
            .to_ascii_lowercase()
            .ends_with(r"\windowspowershell\v1.0\powershell.exe"),
        "{program}"
    );
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
            Some(64 * 1024),
            &CancellationToken::default(),
        )
        .unwrap();
    (service, result)
}

fn scope() -> AgentExecutionScope {
    AgentExecutionScope::for_agent_session(51).unwrap()
}

/// The command's whole output, once it has finished.
fn run(script: &str, tty: bool) -> String {
    let (_service, result) = start(script, tty, YIELD_MS);
    assert_eq!(result.session_id, None, "still running: {:?}", result.output);
    result.output
}

/// Answers a prompt with non-ASCII text through `write_stdin`, as the model
/// does in a PTY session.
fn answer(script: &str) -> String {
    let (service, yielded) = start(script, true, 3_000);
    let Some(session_id) = yielded.session_id else {
        panic!("finished before its input: {:?}", yielded.output);
    };
    let completed = service
        .write_stdin(
            &scope(),
            WriteStdinRequest {
                session_id: AgentExecutionId::from_untrusted(session_id).unwrap(),
                chars: "中文 ü\r".to_string(),
                yield_time_ms: YIELD_MS,
                max_output_bytes: Some(64 * 1024),
            },
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(
        completed.session_id, None,
        "still running: {:?}",
        completed.output
    );
    format!("{}{}", yielded.output, completed.output)
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

#[test]
fn the_shell_resolved_is_windows_powershell_5_1() {
    let output = run("$PSVersionTable.PSVersion.Major", false);
    assert_eq!(output.trim(), "5", "{output:?}");
}

fn the_prefix_sets_the_console_code_page_to_utf8(tty: bool) {
    assert_contains(&run("chcp.com", tty), "65001");
}

#[test]
fn pipe_the_prefix_sets_the_console_code_page_to_utf8() {
    the_prefix_sets_the_console_code_page_to_utf8(false);
}

#[test]
fn pty_the_prefix_sets_the_console_code_page_to_utf8() {
    the_prefix_sets_the_console_code_page_to_utf8(true);
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
        &format!("Write-Output '中文 ü'; {}", ansi_write(ANSI_TEXT)),
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

/// (a): a program that follows the console's code page.
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

/// Temporary, for the reproduction run only: what the console reports.
#[test]
fn probe_code_pages() {
    let script = "\"out=$([Console]::OutputEncoding.CodePage) in=$([Console]::InputEncoding.CodePage) \
                  oe=$($OutputEncoding.CodePage) acp=$([Text.Encoding]::Default.CodePage) \
                  pre=$([Console]::OutputEncoding.GetPreamble().Length)\"; chcp.com";
    let pipe = run(script, false);
    let pty = run(script, true);
    panic!("pipe: {pipe:?}\npty: {pty:?}");
}
