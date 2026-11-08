use std::collections::BTreeMap;

use hanafubuki_agent_foundation::environment::validate_native_environment;

use crate::AgentExecutionError;

#[cfg(not(windows))]
const UNIX_DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// Codex's unified exec defaults: a dumb terminal with no colour, no pager, and
/// UTF-8 text, so a command neither waits on a pager for a keypress nor fills
/// its output with escape sequences. A request's own environment overrides them.
const NON_INTERACTIVE_ENVIRONMENT: [(&str, &str); 9] = [
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("LANG", "C.UTF-8"),
    ("LC_CTYPE", "C.UTF-8"),
    ("LC_ALL", "C.UTF-8"),
    ("COLORTERM", ""),
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GH_PAGER", "cat"),
];

/// Builds the complete environment for an Agent command without inheriting the
/// desktop process environment at spawn time.
pub(crate) fn build_clean_command_environment(
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AgentExecutionError> {
    let mut environment = BTreeMap::new();
    for key in platform_allowlist() {
        if let Some((actual, value)) = read_environment_case_aware(key) {
            environment.insert(actual, value);
        }
    }
    if !contains_key_case_aware(&environment, "PATH") {
        environment.insert("PATH".to_string(), platform_default_path(&environment));
    }
    for (key, value) in NON_INTERACTIVE_ENVIRONMENT {
        remove_key_case_aware(&mut environment, key);
        environment.insert(key.to_string(), value.to_string());
    }
    for (key, value) in overrides {
        remove_key_case_aware(&mut environment, key);
        environment.insert(key.clone(), value.clone());
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
    format!(r"{root}\System32;{root}")
}

#[cfg(windows)]
fn platform_allowlist() -> &'static [&'static str] {
    &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
    ]
}

#[cfg(not(windows))]
fn platform_allowlist() -> &'static [&'static str] {
    &["PATH", "TMPDIR"]
}

fn read_environment_case_aware(name: &str) -> Option<(String, String)> {
    std::env::vars().find(|(key, _)| environment_key_eq(key, name))
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
