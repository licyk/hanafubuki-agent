// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/arg0/src/lib.rs` (dispatching a helper from the command line of
// the same binary) and `codex-rs/sandboxing/src/landlock.rs` (the helper's
// command line) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the helper is selected by its first
// argument, as Codex selects its other helpers (and its Windows sandbox
// wrapper, `--run-as-windows-sandbox`), rather than by argv[0], so the
// Agent's `ExecutorManager` spawns it like any command; it takes its
// writable roots and network switch as flags instead of a JSON policy, and
// a plugin's readable roots likewise; no PATH aliases, `.env` loading or
// other helpers.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The first argument that makes the Agent's binary the sandbox helper.
pub const HELPER_ARG: &str = "--hanafubuki-agent-sandbox";
/// The helper's exit status when it could not confine or start the command.
pub const HELPER_FAILURE_EXIT_CODE: i32 = 126;

const WRITABLE_ROOT_FLAG: &str = "--writable-root";
const READABLE_ROOT_FLAG: &str = "--readable-root";
const RESTRICT_READS_FLAG: &str = "--restrict-reads";
const NETWORK_FLAG: &str = "--allow-network";
const SEPARATOR: &str = "--";

/// This program, once [`helper_dispatch`] found it can serve as the helper.
static HELPER: OnceLock<PathBuf> = OnceLock::new();

/// Runs the sandbox helper and never returns when this process was started
/// as one; otherwise records that this program can serve as the helper.
/// A program that starts the Agent calls it first in `main`, before it
/// starts any thread: without it, a command that asks for a sandbox fails.
pub fn helper_dispatch() {
    let mut args = std::env::args_os();
    if let Some(first) = args.nth(1) {
        let args: Vec<_> = args.collect();
        #[cfg(windows)]
        if let Some(result) = crate::windows::elevated::dispatch(&first, args.clone().into_iter()) {
            match result {
                Ok(code) => std::process::exit(code as i32),
                Err(error) => {
                    eprintln!("hanafubuki-agent sandbox: {error}");
                    std::process::exit(HELPER_FAILURE_EXIT_CODE);
                }
            }
        }
        if first == HELPER_ARG {
            run_helper(args);
        }
    }
    if let Ok(program) = std::env::current_exe() {
        let _ = HELPER.set(program);
    }
}

#[cfg(any(target_os = "linux", windows))]
pub(crate) fn registered_helper() -> Option<&'static PathBuf> {
    HELPER.get()
}

/// What the helper is asked to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HelperRequest {
    pub(crate) writable_roots: Vec<PathBuf>,
    /// `None`: the command reads everything. Otherwise it reads only the
    /// platform's system directories, these roots and its writable ones.
    pub(crate) readable_roots: Option<Vec<PathBuf>>,
    pub(crate) network_access: bool,
    pub(crate) command: Vec<OsString>,
}

impl HelperRequest {
    /// The helper's command line after [`HELPER_ARG`].
    pub(crate) fn to_args(&self) -> Option<Vec<String>> {
        let mut args = Vec::new();
        if self.network_access {
            args.push(NETWORK_FLAG.to_string());
        }
        for root in &self.writable_roots {
            args.push(WRITABLE_ROOT_FLAG.to_string());
            args.push(root.to_str()?.to_string());
        }
        if let Some(readable_roots) = &self.readable_roots {
            args.push(RESTRICT_READS_FLAG.to_string());
            for root in readable_roots {
                args.push(READABLE_ROOT_FLAG.to_string());
                args.push(root.to_str()?.to_string());
            }
        }
        args.push(SEPARATOR.to_string());
        for argument in &self.command {
            args.push(argument.to_str()?.to_string());
        }
        Some(args)
    }

    /// Reads the command line [`Self::to_args`] wrote; anything else is
    /// refused, so a mistake never runs a command less confined.
    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut writable_roots = Vec::new();
        let mut readable_roots: Option<Vec<PathBuf>> = None;
        let mut network_access = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some(NETWORK_FLAG) => network_access = true,
                Some(WRITABLE_ROOT_FLAG) => {
                    let root = args
                        .next()
                        .ok_or_else(|| format!("{WRITABLE_ROOT_FLAG} needs a path"))?;
                    writable_roots.push(PathBuf::from(root));
                }
                Some(RESTRICT_READS_FLAG) => {
                    readable_roots.get_or_insert_with(Vec::new);
                }
                Some(READABLE_ROOT_FLAG) => {
                    let root = args
                        .next()
                        .ok_or_else(|| format!("{READABLE_ROOT_FLAG} needs a path"))?;
                    readable_roots
                        .as_mut()
                        .ok_or_else(|| {
                            format!("{READABLE_ROOT_FLAG} follows {RESTRICT_READS_FLAG}")
                        })?
                        .push(PathBuf::from(root));
                }
                Some(SEPARATOR) => {
                    let command: Vec<OsString> = args.collect();
                    if command.is_empty() {
                        return Err("no command follows `--`".to_string());
                    }
                    return Ok(Self {
                        writable_roots,
                        readable_roots,
                        network_access,
                        command,
                    });
                }
                _ => return Err(format!("unexpected argument {arg:?}")),
            }
        }
        Err("no command follows `--`".to_string())
    }
}

fn run_helper(args: Vec<OsString>) -> ! {
    #[cfg(windows)]
    if args.first().is_some_and(|arg| arg == "--elevated") {
        let args = args.into_iter().skip(1).collect();
        let result = HelperRequest::parse(args).and_then(crate::windows::elevated::run);
        match result {
            Ok(code) => std::process::exit(code as i32),
            Err(error) => {
                eprintln!("hanafubuki-agent sandbox: {error}");
                std::process::exit(HELPER_FAILURE_EXIT_CODE);
            }
        }
    }
    #[cfg(target_os = "linux")]
    if args
        .first()
        .is_some_and(|arg| arg == "--bubblewrap" || arg == "--network-only")
    {
        let mut args = args.into_iter();
        let flag = args.next();
        let error = match HelperRequest::parse(args.collect()) {
            Ok(request) => {
                if flag.as_deref() == Some(std::ffi::OsStr::new("--bubblewrap")) {
                    crate::linux::bubblewrap::run(request)
                } else {
                    crate::linux::network_and_exec(request)
                }
            }
            Err(error) => error,
        };
        eprintln!("hanafubuki-agent sandbox: {error}");
        std::process::exit(HELPER_FAILURE_EXIT_CODE);
    }
    let error = match HelperRequest::parse(args) {
        Ok(request) => confine_and_exec(request),
        Err(error) => error,
    };
    eprintln!("hanafubuki-agent sandbox: {error}");
    std::process::exit(HELPER_FAILURE_EXIT_CODE)
}

#[cfg(target_os = "linux")]
fn confine_and_exec(request: HelperRequest) -> String {
    crate::linux::confine_and_exec(request)
}

/// Windows cannot replace a process: the helper runs the command and exits
/// with its status.
#[cfg(windows)]
fn confine_and_exec(request: HelperRequest) -> String {
    match crate::windows::run(request) {
        // An exit status is a `u32` on Windows; `exit` passes the bits on.
        Ok(status) => std::process::exit(status as i32),
        Err(error) => error,
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn confine_and_exec(_request: HelperRequest) -> String {
    "the sandbox helper runs only on Linux and Windows".to_string()
}

#[cfg(test)]
mod tests;
