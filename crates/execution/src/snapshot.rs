//! Shell startup capture derived from OpenAI Codex 57a808a392 (Apache-2.0),
//! `codex-rs/shell-command/src/startup.rs` and `shell_snapshot_capture.rs`.
//! Snapshots remain in memory: profile exports can contain credentials.

use std::collections::BTreeMap;
use std::path::Path;

use crate::model::ResolvedCommandPlan;
use crate::{AgentExecutionError, ExecCommandRequest, PlatformShell};

pub(crate) struct ShellSnapshot {
    state: String,
    environment: BTreeMap<String, String>,
}

pub(crate) fn kind(program: &str) -> Option<&'static str> {
    match Path::new(program).file_stem()?.to_str()? {
        "bash" => Some("bash"),
        "zsh" => Some("zsh"),
        "sh" | "dash" => Some("sh"),
        _ => None,
    }
}

pub(crate) fn capture_script(kind: &str) -> String {
    // Bash 3.2's `set +o posix` resets expand_aliases; replay shopt after set.
    let (startup, state) = match kind {
        "bash" => (
            "if [ -z \"${BASH_ENV-}\" ] && [ -n \"${HOME-}\" ] && [ -r \"$HOME/.bashrc\" ]; then . \"$HOME/.bashrc\"; fi",
            "declare -f; set +o; shopt -p || true; alias -p",
        ),
        "zsh" => (
            "if [ -n \"${ZDOTDIR-}\" ]; then rc=\"$ZDOTDIR/.zshrc\"; else rc=\"${HOME-}/.zshrc\"; fi; if [ -r \"$rc\" ]; then . \"$rc\"; fi; unset rc",
            "functions; setopt | sed 's/^/setopt /'; alias -L",
        ),
        _ => (
            "if [ -n \"${ENV-}\" ] && [ -r \"$ENV\" ]; then . \"$ENV\"; fi",
            "set +o; if [ -n \"${BASH_VERSION-}\" ]; then declare -f; shopt -p || true; fi; alias",
        ),
    };
    // MSYS exports a POSIX PATH; the next native Windows spawn needs its
    // Windows spelling before it adds the shell's directory again.
    let exports = if cfg!(windows) {
        "env -0 \"PATH=$(cygpath -wp \"$PATH\")\""
    } else {
        "env -0"
    };
    format!("{startup}\nprintf '\\0HANAFUBUKI_SNAPSHOT\\0'\nprintf 'unalias -a 2>/dev/null || true\\n'\n{state}\nprintf '\\0'\n{exports}\n")
}

impl ShellSnapshot {
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        let marker = b"\0HANAFUBUKI_SNAPSHOT\0";
        let start = bytes
            .windows(marker.len())
            .position(|part| part == marker)?
            + marker.len();
        let mut records = bytes[start..].split(|byte| *byte == 0);
        let state = std::str::from_utf8(records.next()?).ok()?.to_string();
        // One native environment value must fit within the OS argument limits.
        if state.len() > 64 * 1024 {
            return None;
        }
        let environment = records
            .filter_map(|record| {
                let (key, value) = std::str::from_utf8(record).ok()?.split_once('=')?;
                (!matches!(key, "PWD" | "OLDPWD" | "SHLVL" | "_"))
                    .then(|| (key.into(), value.into()))
            })
            .collect();
        Some(Self { state, environment })
    }

    pub(crate) fn apply(
        &self,
        request: &ExecCommandRequest,
        plan: &mut ResolvedCommandPlan,
    ) -> Result<(), AgentExecutionError> {
        let mut environment = crate::environment::build_environment_from(
            self.environment.clone(),
            &request.shell_environment,
            &request.environment,
        );
        #[cfg(windows)]
        crate::msys::add_posix_shell_environment(
            &mut environment,
            Path::new(&plan.command[0]),
            std::env::var("MSYSTEM").ok(),
        );
        let mut startup = String::new();
        for name in ["BASH_ENV", "ENV", "ZDOTDIR"] {
            if let Some(value) = environment.remove(name) {
                startup.push_str(&format!("export {name}={}\n", quote(&value)));
            }
        }
        // Startup files were already captured. Replay their state in a non-login
        // shell, then parse the user's command separately so aliases take effect.
        environment.insert("__HANAFUBUKI_SHELL_STATE".into(), self.state.clone());
        environment.insert("__HANAFUBUKI_SHELL_COMMAND".into(), request.cmd.clone());
        let program = plan.command[0].clone();
        let flags = match kind(&program) {
            Some("bash") => vec!["--noprofile", "--norc", "-c"],
            Some("zsh") => vec!["-f", "-c"],
            _ => vec!["-c"],
        };
        let script = format!("{startup}eval \"$__HANAFUBUKI_SHELL_STATE\"\nunset __HANAFUBUKI_SHELL_STATE\neval \"unset __HANAFUBUKI_SHELL_COMMAND\n$__HANAFUBUKI_SHELL_COMMAND\"");
        #[cfg(windows)]
        let script = crate::msys::command_line_script(&script)?;
        plan.command = std::iter::once(program)
            .chain(flags.into_iter().map(str::to_string))
            .chain([script])
            .collect();
        plan.environment = environment;
        Ok(())
    }
}

pub(crate) fn enabled(request: &ExecCommandRequest, plan: &ResolvedCommandPlan) -> bool {
    request.shell_environment.snapshot
        && request.login.unwrap_or(true)
        && request.shell != Some(PlatformShell::PowerShell)
        && request.shell != Some(PlatformShell::Cmd)
        && kind(&plan.command[0]).is_some()
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
