use std::collections::BTreeMap;

use crate::ShellEnvironment;
use hanafubuki_agent_foundation::environment::validate_native_environment;
use hanafubuki_agent_foundation::environment_policy::{current_environment, insert};

use crate::AgentExecutionError;

#[cfg(not(windows))]
const UNIX_DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// Codex's unified exec defaults: a dumb terminal with no colour, no pager, and
/// UTF-8 text, so a command neither waits on a pager for a keypress nor fills
/// its output with escape sequences; and Python in UTF-8, which on Windows
/// otherwise writes a pipe in the ANSI code page. A request's own environment
/// overrides them.
const NON_INTERACTIVE_ENVIRONMENT: [(&str, &str); 12] = [
    ("CODEX_CI", "1"),
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("LANG", "C.UTF-8"),
    ("LC_CTYPE", "C.UTF-8"),
    ("LC_ALL", "C.UTF-8"),
    ("COLORTERM", ""),
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GH_PAGER", "cat"),
    ("PYTHONUTF8", "1"),
    ("PYTHONIOENCODING", "utf-8"),
];

pub(crate) const RUNTIME_IDENTITY_KEYS: &[&str] = &[
    "HANAFUBUKI_AGENT_SESSION_ID",
    "HANAFUBUKI_AGENT_TURN_ID",
    "HANAFUBUKI_AGENT_TOOL_CALL_ID",
    "HANAFUBUKI_AGENT_PERMISSION_MODE",
];

/// Captures the parent's environment before approval, then applies the command
/// defaults and overrides. Spawn installs this snapshot without inheriting again.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn build_command_environment(
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AgentExecutionError> {
    build_environment_from(
        current_environment(),
        &ShellEnvironment::default(),
        overrides,
    )
}

pub(crate) fn build_environment_from(
    inherited: BTreeMap<String, String>,
    settings: &ShellEnvironment,
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AgentExecutionError> {
    #[cfg(windows)]
    let system_root = inherited
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("SYSTEMROOT"))
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| r"C:\Windows".into());
    let mut environment = settings.policy.apply(inherited);
    // Windows PowerShell's CLR needs SystemRoot even without inherited variables.
    #[cfg(windows)]
    if !contains_key_case_aware(&environment, "SYSTEMROOT") {
        environment.insert("SYSTEMROOT".into(), system_root);
    }
    if !contains_key_case_aware(&environment, "PATH") {
        environment.insert("PATH".to_string(), platform_default_path(&environment));
    }
    for (key, value) in NON_INTERACTIVE_ENVIRONMENT {
        remove_key_case_aware(&mut environment, key);
        environment.insert(key.to_string(), value.to_string());
    }
    #[cfg(windows)]
    if !contains_key_case_aware(&environment, "PATHEXT") {
        environment.insert("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into());
    }
    for (key, value) in overrides {
        remove_key_case_aware(&mut environment, key);
        environment.insert(key.clone(), value.clone());
    }
    for key in RUNTIME_IDENTITY_KEYS {
        remove_key_case_aware(&mut environment, key);
    }
    insert(
        &mut environment,
        "HANAFUBUKI_AGENT_VERSION".into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    if let Some(context) = &settings.context {
        insert(
            &mut environment,
            "HANAFUBUKI_AGENT_SESSION_ID".into(),
            context.session_id.to_string(),
        );
        insert(
            &mut environment,
            "HANAFUBUKI_AGENT_TURN_ID".into(),
            context.turn_id.to_string(),
        );
        insert(
            &mut environment,
            "HANAFUBUKI_AGENT_TOOL_CALL_ID".into(),
            context.tool_call_id.clone(),
        );
        if let Some(mode) = &context.permission_mode {
            insert(
                &mut environment,
                "HANAFUBUKI_AGENT_PERMISSION_MODE".into(),
                mode.clone(),
            );
        }
    }
    validate_native_environment(
        environment
            .iter()
            .map(|(key, value)| (key.as_str(), Some(value.as_str()))),
    )
    .map_err(|error| AgentExecutionError::InvalidRequest(error.to_string()))?;
    Ok(environment)
}

#[cfg(not(windows))]
fn platform_default_path(_environment: &BTreeMap<String, String>) -> String {
    UNIX_DEFAULT_PATH.to_string()
}

#[cfg(windows)]
fn platform_default_path(environment: &BTreeMap<String, String>) -> String {
    let root = environment
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("SYSTEMROOT"))
        .map(|(_, value)| value.as_str())
        .unwrap_or(r"C:\Windows");
    format!(r"{root}\System32;{root};{root}\System32\WindowsPowerShell\v1.0")
}

fn contains_key_case_aware(environment: &BTreeMap<String, String>, name: &str) -> bool {
    environment.keys().any(|key| environment_key_eq(key, name))
}

fn remove_key_case_aware(environment: &mut BTreeMap<String, String>, name: &str) {
    if let Some(existing) = environment
        .keys()
        .find(|key| environment_key_eq(key, name))
        .cloned()
    {
        environment.remove(&existing);
    }
}

fn environment_key_eq(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}
