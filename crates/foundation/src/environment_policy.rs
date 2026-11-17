//! Environment selection derived from OpenAI Codex 57a808a392 (Apache-2.0),
//! `codex-rs/protocol/src/shell_environment.rs` and `config_types.rs`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentInherit {
    #[default]
    All,
    Core,
    None,
    Unfiltered,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentPolicy {
    pub inherit: EnvironmentInherit,
    pub ignore_default_excludes: bool,
    pub exclude: Vec<String>,
    pub set: BTreeMap<String, String>,
    pub include_only: Vec<String>,
}

impl Default for EnvironmentPolicy {
    fn default() -> Self {
        Self {
            inherit: EnvironmentInherit::All,
            ignore_default_excludes: true,
            exclude: Vec::new(),
            set: BTreeMap::new(),
            include_only: Vec::new(),
        }
    }
}

impl EnvironmentPolicy {
    pub fn apply(
        &self,
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> BTreeMap<String, String> {
        let mut env: BTreeMap<_, _> = vars
            .into_iter()
            .filter(|(key, _)| self.allows_inherited(key))
            .collect();
        for (key, value) in &self.set {
            insert(&mut env, key.clone(), value.clone());
        }
        if self.inherit != EnvironmentInherit::Unfiltered && !self.include_only.is_empty() {
            env.retain(|key, _| {
                self.include_only
                    .iter()
                    .any(|pattern| matches(pattern, key))
            });
        }
        env
    }

    pub fn allows_inherited(&self, key: &str) -> bool {
        let inherited = match self.inherit {
            EnvironmentInherit::Unfiltered => return true,
            EnvironmentInherit::All => true,
            EnvironmentInherit::None => false,
            EnvironmentInherit::Core => CORE_ENVIRONMENT
                .iter()
                .any(|name| name.eq_ignore_ascii_case(key)),
        };
        inherited
            && (self.ignore_default_excludes
                || !["*KEY*", "*SECRET*", "*TOKEN*"]
                    .iter()
                    .any(|pattern| matches(pattern, key)))
            && !self.exclude.iter().any(|pattern| matches(pattern, key))
    }
}

pub fn current_environment() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

pub fn insert(env: &mut BTreeMap<String, String>, key: String, value: String) {
    if cfg!(windows) {
        env.retain(|name, _| !name.eq_ignore_ascii_case(&key));
    }
    env.insert(key, value);
}

/// Case-insensitive `*` and `?`, as Codex's environment patterns use.
pub fn matches(pattern: &str, name: &str) -> bool {
    let name: Vec<_> = name.to_lowercase().chars().collect();
    let mut previous = vec![false; name.len() + 1];
    previous[0] = true;
    for ch in pattern.to_lowercase().chars() {
        let mut next = vec![false; name.len() + 1];
        next[0] = ch == '*' && previous[0];
        for (index, actual) in name.iter().enumerate() {
            next[index + 1] = if ch == '*' {
                previous[index + 1] || next[index]
            } else {
                previous[index] && (ch == '?' || ch == *actual)
            };
        }
        previous = next;
    }
    previous[name.len()]
}

#[cfg(not(windows))]
pub const CORE_ENVIRONMENT: &[&str] = &[
    "PATH", "SHELL", "TMPDIR", "TEMP", "TMP", "HOME", "LANG", "LC_ALL", "LC_CTYPE", "LOGNAME",
    "USER",
];

#[cfg(windows)]
pub const CORE_ENVIRONMENT: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SHELL",
    "COMSPEC",
    "SYSTEMROOT",
    "WINDIR",
    "SYSTEMDRIVE",
    "USERNAME",
    "USERDOMAIN",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "APPDATA",
    "LOCALAPPDATA",
    "ALLUSERSPROFILE",
    "TEMP",
    "TMP",
    "TMPDIR",
    "POWERSHELL",
    "PWSH",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_IDENTIFIER",
    "NUMBER_OF_PROCESSORS",
    "OS",
    "PSMODULEPATH",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
];

#[cfg(test)]
mod tests;
