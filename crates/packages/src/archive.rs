// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/plugin_bundle_archive.rs` (a package bundle's
// tar.gz: the `package/` root an `npm pack` writes, the total extracted size
// it may not pass, the entries it refuses) at commit `bfe3c82570`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: only the unpacking half is ported (this
// project has no producer of bundles yet); the archive is read from memory,
// and a failure is its message.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

/// Unpacks the gzipped tar `bytes` into `destination`, refusing entries that
/// are links, of another type, or that would leave it, and stopping once
/// their total size passes `max_total_bytes`.
pub(crate) fn unpack_tar_gz(
    bytes: &[u8],
    destination: &Path,
    max_total_bytes: u64,
) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create the extraction directory: {error}"))?;
    let mut archive = Archive::new(GzDecoder::new(io::Cursor::new(bytes)));
    let entries = archive
        .entries()
        .map_err(|error| format!("failed to read the archive: {error}"))?;
    let mut extracted_bytes = 0u64;
    for entry in entries {
        let mut entry =
            entry.map_err(|error| format!("failed to read an archive entry: {error}"))?;
        let entry_type = entry.header().entry_type();
        let entry_size = entry.size();
        let entry_path = entry
            .path()
            .map_err(|error| format!("failed to read an archive entry's path: {error}"))?
            .into_owned();
        let output_path = output_path(destination, &entry_path)?;
        if entry_type.is_dir() {
            fs::create_dir_all(&output_path)
                .map_err(|error| format!("failed to create a directory: {error}"))?;
            continue;
        }
        if entry_type.is_file() {
            extracted_bytes = extracted_bytes
                .checked_add(entry_size)
                .filter(|total| *total <= max_total_bytes)
                .ok_or_else(|| {
                    format!(
                        "the archive's extracted size passes the maximum of {max_total_bytes} bytes"
                    )
                })?;
            let parent = output_path
                .parent()
                .ok_or_else(|| format!("{} has no parent", output_path.display()))?;
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create a directory: {error}"))?;
            entry
                .unpack(&output_path)
                .map_err(|error| format!("failed to unpack an entry: {error}"))?;
            continue;
        }
        if entry_type.is_hard_link() || entry_type.is_symlink() {
            return Err(format!(
                "the archive's entry `{}` is a link",
                entry_path.display()
            ));
        }
        return Err(format!(
            "the archive's entry `{}` has an unsupported type",
            entry_path.display()
        ));
    }
    Ok(())
}

/// The entry's path under `destination`: normal components only, at least
/// one of them.
fn output_path(destination: &Path, entry_path: &Path) -> Result<PathBuf, String> {
    let mut path = destination.to_path_buf();
    let mut components = 0;
    for component in entry_path.components() {
        match component {
            Component::Normal(component) => {
                components += 1;
                path.push(component);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "the archive's entry `{}` leaves the extraction directory",
                    entry_path.display()
                ))
            }
        }
    }
    if components == 0 {
        return Err("the archive's entry has an empty path".to_string());
    }
    Ok(path)
}
