//! Derived from OpenAI Codex 57a808a392 (Apache-2.0),
//! `codex-rs/rmcp-client/src/utils.rs` (local stdio environment selection).

use std::collections::HashMap;

use hanafubuki_agent_foundation::environment_policy::current_environment;
use hanafubuki_agent_foundation::paths::normalize_path;

use crate::{McpError, McpStdioConfig};

pub(super) fn inherited(
    config: &McpStdioConfig,
) -> Result<HashMap<String, Option<String>>, McpError> {
    let parent = current_environment();
    let mut environment: HashMap<_, _> = parent
        .iter()
        .filter(|(key, _)| {
            BASIC
                .iter()
                .copied()
                .chain(config.environment_variables.iter().map(String::as_str))
                .any(|name| {
                    if cfg!(windows) {
                        name.eq_ignore_ascii_case(key)
                    } else {
                        name == key.as_str()
                    }
                })
        })
        .map(|(key, value)| (key.clone(), Some(value.clone())))
        .collect();
    for name in CA_VARIABLES {
        let Some((key, value)) = parent.iter().find(|(key, value)| {
            !value.is_empty()
                && (if cfg!(windows) {
                    key.eq_ignore_ascii_case(name)
                } else {
                    key.as_str() == *name
                })
        }) else {
            continue;
        };
        // Resolve against the Agent's cwd, before the server changes directory.
        let path = normalize_path(value.into()).map_err(|_| {
            McpError::InvalidConfig("cannot resolve an inherited certificate path".into())
        })?;
        environment.insert(key.clone(), Some(path.to_string_lossy().into_owned()));
    }
    Ok(environment)
}

#[cfg(unix)]
const BASIC: &[&str] = &[
    "HOME",
    "LOGNAME",
    "PATH",
    "SHELL",
    "USER",
    "__CF_USER_TEXT_ENCODING",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TZ",
];
#[cfg(windows)]
const BASIC: &[&str] = hanafubuki_agent_foundation::environment_policy::CORE_ENVIRONMENT;
#[cfg(not(any(unix, windows)))]
const BASIC: &[&str] = &["PATH", "HOME", "TMPDIR"];

const CA_VARIABLES: &[&str] = &[
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
    "GIT_SSL_CAINFO",
    "CARGO_HTTP_CAINFO",
    "PIP_CERT",
    "BUNDLE_SSL_CA_CERT",
    "npm_config_cafile",
    "NPM_CONFIG_CAFILE",
];
