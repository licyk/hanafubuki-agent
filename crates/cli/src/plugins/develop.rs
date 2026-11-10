//! The plugin author's commands: `plugins validate` checks a plugin's
//! directory, and with `--run` starts it in a throwaway home; `plugins dev`
//! installs it and installs it again each time its files change.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use hanafubuki_agent::{
    validate_package, AgentDirs, AgentHandle, AgentPackageInstall, AgentPackageSource,
    AgentPackageStatus, AgentPackageValidation, GuestState,
};
use notify::{RecursiveMode, Watcher};

use super::code::{confirm_grant, grant_permissions, permissions_text, print_guest, settled};
use super::print_json;
use crate::agent::{block_on, with_agent, with_agent_in};
use crate::cli::GlobalArgs;
use crate::exit::Failure;

/// How long the files must be quiet after a change before the plugin is
/// installed again: one build or save writes many.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// `plugins validate DIR [--run] [--json]`: fails when anything is found.
pub(crate) fn validate(
    global: &GlobalArgs,
    dir: &Path,
    run: bool,
    json: bool,
) -> Result<(), Failure> {
    let dir = absolute(dir)?;
    let validation = validate_package(&dir).map_err(|error| {
        Failure::new(format!(
            "{} holds no plugin an install reads: {error}",
            dir.display()
        ))
    })?;
    let started = match run {
        true => Some(run_once(global, &dir)?),
        false => None,
    };
    let running = started.as_ref().is_none_or(|package| {
        package
            .guest
            .as_ref()
            .is_none_or(|guest| guest.status.state == GuestState::Running)
    });
    let clean = validation.issues.is_empty()
        && started.as_ref().is_none_or(|package| {
            package.issues.is_empty()
                && package
                    .guest
                    .as_ref()
                    .is_none_or(|guest| guest.status.issues.is_empty())
        });
    if json {
        print_json(&serde_json::json!({ "validation": validation, "started": started }))?;
    } else {
        print_validation(&validation);
        if let Some(package) = &started {
            println!("Started in a throwaway home:");
            print_guest(package);
        }
    }
    match (clean, running) {
        (true, true) => Ok(()),
        (_, false) => Err(Failure::new("its guest did not start")),
        (false, true) => Err(Failure::new("the plugin has issues")),
    }
}

fn print_validation(validation: &AgentPackageValidation) {
    let version = validation.version.as_deref().unwrap_or("no version");
    let kind = match validation.code {
        true => "with code",
        false => "content only",
    };
    println!(
        "{} ({version}, {:?} format, {kind}): {}",
        validation.name,
        validation.format,
        validation.manifest.display()
    );
    if validation.issues.is_empty() {
        println!("  No issues.");
    }
    for issue in &validation.issues {
        println!("  issue:    {issue}");
    }
}

/// Installs the plugin in `dir` into a throwaway home (its own configuration
/// and data directory, removed after), grants what its code asks for, and
/// returns it once its guest settled.
fn run_once(global: &GlobalArgs, dir: &Path) -> Result<AgentPackageStatus, Failure> {
    let home =
        tempfile::tempdir().map_err(|error| Failure::new(format!("a throwaway home: {error}")))?;
    let dirs = AgentDirs {
        config_dir: home.path().join("config"),
        data_dir: home.path().join("data"),
    };
    with_agent_in(global, dirs, |agent| {
        let install = install(agent, dir)?;
        if let Some(permissions) = install.permissions {
            grant_permissions(agent, &install.package.id, permissions)?;
        }
        settled(agent, &install.package.id)
    })
}

/// `plugins dev DIR [--grant]`: installs the plugin, then installs it again
/// after each change to its files, until Ctrl+C. It stays installed.
pub(crate) fn dev(global: &GlobalArgs, dir: &Path, grant: bool) -> Result<(), Failure> {
    let dir = absolute(dir)?;
    with_agent(global, |agent| {
        let (events, received) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(watch_sender(events.clone(), &dir))
            .map_err(|error| Failure::new(format!("the directory cannot be watched: {error}")))?;
        watcher
            .watch(&dir, RecursiveMode::Recursive)
            .map_err(|error| Failure::new(format!("{}: {error}", dir.display())))?;
        watch_interrupt(agent, events);
        let id = reinstall(agent, &dir, grant, None)?;
        println!("Watching {} (Ctrl+C stops).", dir.display());
        while wait_for_change(&received) {
            if let Err(failure) = reinstall(agent, &dir, grant, Some(&id)) {
                eprintln!("error: {}", failure.message);
            }
        }
        println!("Stopped. `{id}` stays installed; `plugins remove {id}` removes it.");
        Ok(())
    })
}

enum DevEvent {
    Changed,
    Interrupted,
}

/// Whether a change came (once the files are quiet), not Ctrl+C.
fn wait_for_change(received: &Receiver<DevEvent>) -> bool {
    match received.recv() {
        Ok(DevEvent::Changed) => {}
        Ok(DevEvent::Interrupted) | Err(_) => return false,
    }
    loop {
        match received.recv_timeout(DEBOUNCE) {
            Ok(DevEvent::Changed) => {}
            Ok(DevEvent::Interrupted) | Err(RecvTimeoutError::Disconnected) => return false,
            Err(RecvTimeoutError::Timeout) => return true,
        }
    }
}

/// Installs the plugin in `dir` (again, when `previous` is its id), asks
/// for what its code asks for and was not granted, and prints how its guest
/// runs. Returns its id.
fn reinstall(
    agent: &AgentHandle,
    dir: &Path,
    grant: bool,
    previous: Option<&str>,
) -> Result<String, Failure> {
    let install = install(agent, dir)?;
    let id = install.package.id.clone();
    match previous {
        Some(previous) if previous != id => println!(
            "Installed `{id}` (its name changed; `{previous}` stays installed) (version {}).",
            install.package.version
        ),
        Some(_) => println!("Reinstalled `{id}` (version {}).", install.package.version),
        None => println!("Installed `{id}` (version {}).", install.package.version),
    }
    for issue in &install.issues {
        println!("  issue:    {issue}");
    }
    if let Some(permissions) = install.permissions {
        if grant || confirm_grant(&id, &permissions)? {
            let shown = permissions_text(&permissions);
            grant_permissions(agent, &id, permissions)?;
            println!("Granted {shown}.");
        } else {
            println!("Not granted: its guest waits for `plugins grant {id}`.");
        }
    }
    print_guest(&settled(agent, &id)?);
    Ok(id)
}

fn install(agent: &AgentHandle, dir: &Path) -> Result<AgentPackageInstall, Failure> {
    let source = AgentPackageSource::Directory {
        path: dir.to_path_buf(),
        marketplace: None,
    };
    block_on(
        agent,
        |agent| async move { agent.install_package(source).await },
    )
}

/// What the watcher sends: a change to a file in `dir`, but not in its
/// `.git`.
fn watch_sender(
    events: Sender<DevEvent>,
    dir: &Path,
) -> impl Fn(notify::Result<notify::Event>) + Send + 'static {
    let git = dir.join(".git");
    move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if event.kind.is_access() || event.paths.iter().all(|path| path.starts_with(&git)) {
            return;
        }
        let _ = events.send(DevEvent::Changed);
    }
}

fn watch_interrupt(agent: &AgentHandle, events: Sender<DevEvent>) {
    agent.runtime().handle().spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = events.send(DevEvent::Interrupted);
        }
    });
}

fn absolute(dir: &Path) -> Result<PathBuf, Failure> {
    if !dir.is_dir() {
        return Err(Failure::usage(format!(
            "{} is not a directory",
            dir.display()
        )));
    }
    std::path::absolute(dir).map_err(|error| Failure::usage(format!("{}: {error}", dir.display())))
}
