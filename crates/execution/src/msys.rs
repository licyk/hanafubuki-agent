//! The POSIX shell on Windows: the bash (or sh) of Git for Windows or MSYS2,
//! both built on the MSYS runtime.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::PathKey;

use crate::model::AgentExecutionError;
use crate::resolver::{canonical_executable, environment_value};

const SHELLS: [&str; 2] = ["bash.exe", "sh.exe"];

/// Where a POSIX shell is looked for, in order: bash before sh at each step.
pub(crate) struct PosixShellSearch {
    /// The Agent's own `PATH` when it runs inside an MSYS2 or Git Bash shell
    /// (`MSYSTEM` is set), whose shell comes first.
    pub(crate) msys_path: Option<OsString>,
    /// The command's `PATH`.
    pub(crate) path: OsString,
    /// Directories whose `bash.exe` is WSL's launcher, not a shell of this
    /// machine; so is any under a `WindowsApps` directory.
    pub(crate) wsl_directories: Vec<PathBuf>,
    /// The usual installations of Git for Windows and MSYS2.
    pub(crate) roots: Vec<PathBuf>,
}

impl PosixShellSearch {
    /// The search for a command with `environment`, and with the Agent's
    /// own environment as it is now.
    #[cfg(windows)]
    pub(crate) fn of(environment: &BTreeMap<String, String>) -> Self {
        let value = |name| environment_value(environment, name);
        let system_root = PathBuf::from(value("SYSTEMROOT").unwrap_or(r"C:\Windows"));
        let roots = [
            value("ProgramFiles").map(|directory| Path::new(directory).join("Git")),
            value("ProgramFiles(x86)").map(|directory| Path::new(directory).join("Git")),
            value("LOCALAPPDATA").map(|directory| Path::new(directory).join(r"Programs\Git")),
            value("SystemDrive").map(|drive| PathBuf::from(format!(r"{drive}\msys64"))),
        ];
        Self {
            msys_path: std::env::var_os("MSYSTEM").and(std::env::var_os("PATH")),
            path: value("PATH").unwrap_or_default().into(),
            wsl_directories: ["System32", "SysWOW64", "Sysnative"]
                .map(|directory| system_root.join(directory))
                .into(),
            roots: roots.into_iter().flatten().collect(),
        }
    }

    pub(crate) fn find(&self) -> Result<PathBuf, AgentExecutionError> {
        let steps = [
            self.msys_path
                .iter()
                .flat_map(std::env::split_paths)
                .collect::<Vec<_>>(),
            std::env::split_paths(&self.path).collect(),
            self.roots
                .iter()
                .map(|root| root.join("usr").join("bin"))
                .collect(),
        ];
        for directories in steps {
            for name in SHELLS {
                if let Some(shell) = directories
                    .iter()
                    .find_map(|directory| self.shell_in(directory, name))
                {
                    return Ok(shell);
                }
            }
        }
        Err(self.not_found())
    }

    fn shell_in(&self, directory: &Path, name: &str) -> Option<PathBuf> {
        let shell = canonical_executable(&directory.join(name)).ok()?;
        if self.is_wsl_launcher(&shell) {
            return None;
        }
        Some(shell_behind_git_launcher(&shell, name).unwrap_or(shell))
    }

    fn is_wsl_launcher(&self, shell: &Path) -> bool {
        let key = PathKey::of(shell);
        let windows_apps = PathKey::of(Path::new("WindowsApps"));
        self.wsl_directories
            .iter()
            .any(|directory| key.starts_with(&PathKey::of(directory)))
            || shell
                .components()
                .any(|component| PathKey::of(Path::new(component.as_os_str())) == windows_apps)
    }

    fn not_found(&self) -> AgentExecutionError {
        let roots = self
            .roots
            .iter()
            .map(|root| root.join("usr").join("bin").display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        AgentExecutionError::Resolution(format!(
            "no POSIX shell was found: looked for bash.exe, then sh.exe, of Git for Windows or \
             MSYS2 {}in the command's PATH (leaving out WSL's launchers in System32 and \
             WindowsApps) and in {roots}; shell \"power_shell\" is available",
            if self.msys_path.is_some() {
                "in the PATH of the MSYS2 or Git Bash shell the Agent runs in, "
            } else {
                ""
            }
        ))
    }
}

/// Git for Windows' `bin\bash.exe` (and `bin\sh.exe`) only starts
/// `usr\bin\bash.exe` as its own child, so the shell itself is run instead:
/// a pipe-mode command is put in its Job Object only after it started, so
/// a child the launcher started by then would outlive a stop.
fn shell_behind_git_launcher(shell: &Path, name: &str) -> Option<PathBuf> {
    let bin = shell.parent()?;
    if PathKey::of(Path::new(bin.file_name()?)) != PathKey::of(Path::new("bin")) {
        return None;
    }
    canonical_executable(&bin.parent()?.join("usr").join("bin").join(name)).ok()
}

/// What a POSIX shell on Windows needs beside the clean environment; a
/// request's own variables win.
///
/// - The shell's directory first on `PATH`, where the MSYS programs (`ls`,
///   `uname`) are: a shell that is not a login shell keeps the `PATH` it
///   is given, a login shell builds its own from it.
/// - `CHERE_INVOKING=1`: MSYS2's `/etc/profile` otherwise changes to
///   `$HOME` in a login shell.
/// - `MSYS2_PATH_TYPE=inherit`: MSYS2's login shell otherwise keeps only
///   the system directories of the Windows `PATH` (Git for Windows'
///   inherits it by default).
/// - `MSYSTEM` of the MSYS2 or Git Bash shell the Agent runs in, if it does.
pub(crate) fn add_posix_shell_environment(
    environment: &mut BTreeMap<String, String>,
    shell: &Path,
    msystem: Option<String>,
) {
    let directory = shell
        .parent()
        .and_then(Path::to_str)
        .unwrap_or_default()
        .to_string();
    let path_key = environment
        .keys()
        .find(|key| key.eq_ignore_ascii_case("PATH"))
        .cloned()
        .unwrap_or_else(|| "PATH".to_string());
    let path = match environment.get(&path_key) {
        Some(path) if !path.is_empty() => format!("{directory};{path}"),
        _ => directory,
    };
    environment.insert(path_key, path);
    let defaults = [
        ("CHERE_INVOKING", Some("1".to_string())),
        ("MSYS2_PATH_TYPE", Some("inherit".to_string())),
        ("MSYSTEM", msystem),
    ];
    for (key, value) in defaults {
        if let Some(value) = value {
            if environment_value(environment, key).is_none() {
                environment.insert(key.to_string(), value);
            }
        }
    }
}

/// The longest script, in UTF-16 code units, that is run: the MSYS runtime
/// cuts a command-line argument longer than 8186 characters short without a
/// word (measured on Git for Windows' and MSYS2's runtimes, with text of the
/// Basic Multilingual Plane), so a longer one is refused rather than run cut.
pub(crate) const MAX_SCRIPT_UNITS: usize = 8_000;

/// The script as it must be spelled on the command line for the MSYS
/// runtime to hand it to bash unchanged.
///
/// A program built on that runtime splits its Windows command line itself,
/// not as the Microsoft C runtime does: outside quotes it splits at a
/// newline, expands `*` and takes `'` as a quote; inside quotes it reads
/// `\\` as one backslash. The command line is built by a quoter that
/// follows the C runtime (the standard library's for pipes, the
/// pseudo-terminal's, the Windows sandbox helper's), and each of them
/// quotes an argument with a space or tab and doubles only the backslashes
/// before a `"` or the end. So a script without a space or tab gets one in
/// front (bash ignores it), and every other run of backslashes is doubled.
pub(crate) fn command_line_script(script: &str) -> Result<String, AgentExecutionError> {
    let spaced = !script.contains([' ', '\t']);
    if script.encode_utf16().count() + usize::from(spaced) > MAX_SCRIPT_UNITS {
        return Err(AgentExecutionError::InvalidRequest(format!(
            "a POSIX shell on Windows takes a script of at most {MAX_SCRIPT_UNITS} characters, \
             since the MSYS runtime cuts a longer one short: write it to a file and run that, \
             or use shell \"power_shell\""
        )));
    }
    let mut spelled = String::with_capacity(script.len() + 1);
    if spaced {
        spelled.push(' ');
    }
    let mut backslashes = 0;
    for character in script.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        let run = if character == '"' {
            backslashes
        } else {
            backslashes * 2
        };
        spelled.extend(std::iter::repeat_n('\\', run));
        backslashes = 0;
        spelled.push(character);
    }
    spelled.extend(std::iter::repeat_n('\\', backslashes));
    Ok(spelled)
}

#[cfg(test)]
mod tests;
