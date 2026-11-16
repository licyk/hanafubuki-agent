use super::resume_hint;
use hanafubuki_agent::AgentSessionId;
use std::path::Path;

#[test]
fn exiting_without_a_session_has_no_resume_command() {
    assert_eq!(resume_hint(None, Some(Path::new("custom home"))), None);
}

#[test]
fn a_session_has_a_direct_resume_command() {
    assert_eq!(
        resume_hint(Some(AgentSessionId::new(52)), None).as_deref(),
        Some("To continue this session, run:\n  hanafubuki-agent resume 52")
    );
}

#[cfg(unix)]
#[test]
fn a_custom_data_directory_is_one_literal_shell_argument() {
    use clap::Parser;

    let directory = "/tmp/Agent's home/$HOME `echo unexpected`";
    let hint = resume_hint(Some(AgentSessionId::new(52)), Some(Path::new(directory))).unwrap();
    let command = hint.lines().nth(1).unwrap();
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("set -- {command}; printf '%s\\0' \"$@\""))
        .output()
        .unwrap();
    assert!(output.status.success());
    let arguments = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| std::str::from_utf8(arg).unwrap());
    let cli = crate::cli::Cli::try_parse_from(arguments).unwrap();
    assert_eq!(cli.global.data_dir.as_deref(), Some(Path::new(directory)));
    assert!(
        matches!(cli.command, Some(crate::cli::Command::Resume { session: Some(id), .. }) if id == "52")
    );
}

#[cfg(windows)]
#[test]
fn a_custom_data_directory_is_quoted_for_powershell() {
    assert_eq!(
        resume_hint(Some(AgentSessionId::new(52)), Some(Path::new("C:\\Agent's home\\$data"))).as_deref(),
        Some("To continue this session, run:\n  hanafubuki-agent resume 52 --data-dir 'C:\\Agent''s home\\$data'")
    );
}
