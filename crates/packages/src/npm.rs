// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/npm_source.rs` (an npm plugin source: `npm
// pack --ignore-scripts` into a staging directory, its one archive, its
// bounds, the `package/` root of the extracted package and its name) at
// commit `bfe3c82570`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: `npm` runs through the shared
// [`crate::program`] runner (the Agent's `ExecutorManager`) rather than
// `std::process::Command`, so it reaches the environment and proxy of the
// Agent's processes; the package is unpacked by
// [`crate::archive::unpack_tar_gz`]; the package's paths are resolved for
// this platform.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_process::executor::ExecutorManager;
use serde::Deserialize;

use crate::archive::unpack_tar_gz;
use crate::program::{self, ProgramRequest};

const NPM_TIMEOUT_MS: u64 = 10 * 60 * 1_000;
const NPM_OUTPUT_CAP: usize = 4 * 1024 * 1024;
/// The archive `npm pack` writes.
const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;
/// What the archive may hold once unpacked.
const MAX_EXTRACTED_BYTES: u64 = 250 * 1024 * 1024;
/// The directory an npm archive's files are under.
const PACKAGE_ROOT: &str = "package";

/// The program that fetches a package on this platform: `npm` is a shell
/// script on Windows and is reached by its `npm.cmd` there.
fn npm_program() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}

/// Packs `package` (`@version`, from `registry` when given) into `staging`
/// with `npm pack --ignore-scripts`, unpacks it there and returns the
/// package's root: `staging/extracted/package`.
pub(crate) fn materialize(
    executor: &ExecutorManager,
    staging: &Path,
    package: &str,
    version: Option<&str>,
    registry: Option<&str>,
) -> Result<PathBuf, String> {
    let staging = absolute(staging);
    let spec = match version {
        Some(version) => format!("{package}@{version}"),
        None => package.to_string(),
    };
    let mut command = vec![
        npm_program().to_string(),
        "pack".to_string(),
        "--ignore-scripts".to_string(),
        "--pack-destination".to_string(),
        staging.display().to_string(),
    ];
    if let Some(registry) = registry {
        command.push("--registry".to_string());
        command.push(registry.to_string());
    }
    command.push("--".to_string());
    command.push(spec);
    // Its own cache, so a pack writes nothing into the user's npm state.
    let env = BTreeMap::from([(
        "npm_config_cache".to_string(),
        Some(staging.join(".npm-cache").display().to_string()),
    )]);
    let (exit_code, stdout, stderr) = program::run(
        executor,
        ProgramRequest {
            name: "npm",
            command: &command,
            cwd: &staging,
            env: &env,
            timeout_ms: NPM_TIMEOUT_MS,
            output_cap: NPM_OUTPUT_CAP,
        },
    )?;
    if exit_code != Some(0) {
        let stderr = String::from_utf8_lossy(&stderr);
        let stdout = String::from_utf8_lossy(&stdout);
        let stdout = stdout.trim();
        let stderr = stderr.trim();
        return Err(match (stderr.is_empty(), stdout.is_empty()) {
            (false, _) => format!("npm pack failed: {stderr}"),
            (true, false) => format!("npm pack failed: {stdout}"),
            (true, true) => format!("npm pack exited with {exit_code:?}"),
        });
    }
    let archive = find_archive(&staging)?;
    let bytes = fs::read(&archive).map_err(|error| {
        format!(
            "the npm package archive {} cannot be read: {error}",
            archive.display()
        )
    })?;
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(format!(
            "the npm package archive is {} bytes, passing the maximum of {MAX_ARCHIVE_BYTES}",
            bytes.len()
        ));
    }
    let extracted = staging.join("extracted");
    unpack_tar_gz(&bytes, &extracted, MAX_EXTRACTED_BYTES)
        .map_err(|error| format!("the npm package cannot be unpacked: {error}"))?;
    let root = extracted.join(PACKAGE_ROOT);
    if !root.is_dir() {
        return Err(format!(
            "npm pack wrote no `{PACKAGE_ROOT}` directory into the archive"
        ));
    }
    validate_name(&root, package)?;
    Ok(root)
}

/// The one archive `npm pack` left in `staging`.
fn find_archive(staging: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(staging)
        .map_err(|error| format!("the npm pack directory cannot be read: {error}"))?;
    let archives = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|file_type| file_type.is_file())
                && entry.path().extension().is_some_and(|kind| kind == "tgz")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    match archives.len() {
        1 => Ok(archives.into_iter().next().unwrap_or_default()),
        count => Err(format!(
            "npm pack wrote {count} package archives where exactly one is expected"
        )),
    }
}

/// The package's `package.json` names `package`.
fn validate_name(root: &Path, package: &str) -> Result<(), String> {
    #[derive(Deserialize)]
    struct Metadata {
        name: String,
    }

    let path = root.join("package.json");
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("{} cannot be read: {error}", path.display()))?;
    let metadata = serde_json::from_str::<Metadata>(&contents)
        .map_err(|error| format!("{} cannot be read: {error}", path.display()))?;
    if metadata.name != package {
        return Err(format!(
            "the npm package is named `{}` where `{package}` was asked for",
            metadata.name
        ));
    }
    Ok(())
}

/// `path` in its absolute form; as it is when it cannot be made one.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}
