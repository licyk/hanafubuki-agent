//! The install flow of `update` (and of the prompt's "Update now"): a
//! fresh lookup, then the release installed as the installation's action
//! says. Progress goes to standard error, rewritten in place on a
//! terminal; the result is the last line, on standard output.

use std::io::{IsTerminal, Write};

use hanafubuki_agent_update::{
    asset_name, compare, current_version, install_blocking, run_cargo, this_target, Comparison,
    Installation, Progress, ReleaseClient, UpdateAction, UpdateError, Version,
};

use super::{failure, manual};
use crate::exit::{Failure, FAILURE, SUCCESS};

/// Looks the latest release up and installs it when it is newer, or the
/// same version with `force`. Exits 0 when it did that or there was
/// nothing to do, 1 when it could not (the manual cases included).
pub(crate) fn update(
    installation: &Installation,
    client: &ReleaseClient,
    force: bool,
) -> Result<u8, Failure> {
    let release = client.latest_blocking().map_err(failure)?;
    let latest = &release.version;
    let current = current_version();
    match compare(latest, &current) {
        Comparison::Available => {}
        Comparison::UpToDate if force => {}
        Comparison::UpToDate => {
            println!("hanafubuki-agent {latest} is the latest release; nothing to do.");
            return Ok(SUCCESS);
        }
        Comparison::Ahead if force => {
            return Err(failure(UpdateError::Older {
                current,
                latest: latest.clone(),
            }))
        }
        Comparison::Ahead => {
            println!(
                "This build ({current}) is newer than the latest release ({latest}); nothing \
                 to install."
            );
            return Ok(SUCCESS);
        }
    }
    match installation.action() {
        UpdateAction::Replace => {
            let executable = installation.executable().native_text();
            if current == *latest {
                eprintln!("Reinstalling hanafubuki-agent {latest} at {executable}");
            } else {
                eprintln!("Updating hanafubuki-agent {current} → {latest} at {executable}");
            }
            let archive = this_target()
                .map(|target| asset_name(latest, target))
                .unwrap_or_default();
            let mut lines = Lines::new(archive, latest.clone());
            let installed = install_blocking(client, installation, &release, force, &mut |step| {
                lines.show(step)
            });
            let installed = installed.map_err(|error| {
                lines.fail();
                failure(error)
            })?;
            if installed.already {
                println!(
                    "hanafubuki-agent at {executable} is already {} (another process \
                     updated it).",
                    installed.version
                );
                return Ok(SUCCESS);
            }
            eprintln!("done");
            println!("{}", updated(latest, &current));
            if let Some(backup) = installed.backup.as_ref().and_then(|path| path.file_name()) {
                println!(
                    "The previous executable is kept as {} until it no longer runs; the next \
                     update or start removes it.",
                    backup.to_string_lossy()
                );
            }
            Ok(SUCCESS)
        }
        UpdateAction::Cargo { command } => {
            let shown = command.join(" ");
            let force_flag = if force { " --force" } else { "" };
            eprintln!("Running {shown}{force_flag}");
            let version = run_cargo(installation, &command, force).map_err(failure)?;
            println!("{}", updated(&version, &current));
            Ok(SUCCESS)
        }
        UpdateAction::Manual(reason) => {
            println!("{}", manual(installation, reason, latest));
            Ok(FAILURE)
        }
    }
}

fn updated(version: &Version, current: &Version) -> String {
    format!(
        "Updated to {version}. The next start of hanafubuki-agent runs it; running ones keep \
         {current} until they restart."
    )
}

/// The progress lines on standard error. On a terminal the download line
/// is rewritten in place; otherwise only its first state and its end are
/// printed.
struct Lines {
    archive: String,
    version: Version,
    terminal: bool,
    /// The download's size, once it started.
    total: Option<u64>,
    /// The width of the download line last drawn on a terminal.
    drawn: usize,
    /// Whether a line is open (the cursor after its text).
    open: bool,
}

impl Lines {
    fn new(archive: String, version: Version) -> Self {
        Self {
            archive,
            version,
            terminal: std::io::stderr().is_terminal(),
            total: None,
            drawn: 0,
            open: false,
        }
    }

    fn show(&mut self, step: Progress) {
        match step {
            Progress::Downloading { done, total } => {
                let line = format!(
                    "Downloading {}: {} of {} MiB ({}%)",
                    self.archive,
                    mebibytes(done),
                    mebibytes(total),
                    (done * 100).checked_div(total).unwrap_or(100)
                );
                if self.terminal {
                    self.redraw(&line);
                } else if self.total.is_none() {
                    eprintln!("{line}");
                }
                self.total = Some(total);
            }
            Progress::Verifying => {
                let line = format!(
                    "Downloading {}: {} MiB: done",
                    self.archive,
                    mebibytes(self.total.unwrap_or_default())
                );
                if self.terminal {
                    self.redraw(&line);
                    eprintln!();
                } else {
                    eprintln!("{line}");
                }
                self.begin("Verifying the SHA-256 digest: ");
            }
            Progress::Checking => {
                eprintln!("ok");
                self.begin("Checking the new executable: ");
            }
            Progress::Installing => {
                eprintln!("hanafubuki-agent {}", self.version);
                self.begin("Installing: ");
            }
        }
    }

    /// Ends an open line before the error that follows it.
    fn fail(&mut self) {
        if self.open {
            eprintln!("failed");
        } else if self.drawn > 0 {
            eprintln!();
        }
        self.open = false;
        self.drawn = 0;
    }

    fn begin(&mut self, text: &str) {
        eprint!("{text}");
        let _ = std::io::stderr().flush();
        self.drawn = 0;
        self.open = true;
    }

    /// `line` over the one drawn before, padded to its width.
    fn redraw(&mut self, line: &str) {
        let width = line.chars().count();
        let padding = " ".repeat(self.drawn.saturating_sub(width));
        eprint!("\r{line}{padding}");
        let _ = std::io::stderr().flush();
        self.drawn = width;
    }
}

fn mebibytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}
