// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/hooks/src/engine/plugin_fingerprint.rs` (a package's content
// hash: every entry's relative path, kind, length and bytes, skipping
// Python's generated files) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: relative paths are hashed with `/`
// separators, so a package hashes the same on every platform; no
// cancellation.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

/// The content hash of the package at `root`, `sha256:<hex>`. It detects a
/// changed source, not tampering inside the skipped runtime artifacts
/// (`__pycache__`, `.venv`, `*.pyc`, `*.egg-info`). A file that changes
/// while it is read fails the hash rather than giving a partial one.
pub fn content_hash(root: &Path) -> io::Result<String> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(io::Error::other("plugin root is not a directory"));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut hash = Sha256::new();
    hash.update(b"codex-plugin-bundle-v1");
    let mut buffer = [0_u8; 64 * 1024];
    while let Some(path) = pending.pop() {
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        if relative
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_skipped)
        {
            continue;
        }
        let name = portable(relative);
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            hash.update(b"directory");
            let mut entries = fs::read_dir(&path)?.collect::<io::Result<Vec<_>>>()?;
            entries.sort_by_key(fs::DirEntry::file_name);
            pending.extend(entries.iter().rev().map(fs::DirEntry::path));
        } else if metadata.is_file() {
            hash.update(b"file");
            let length = metadata.len();
            hash.update(length.to_le_bytes());
            let mut file = fs::File::open(&path)?.take(length.saturating_add(1));
            let mut read = 0_u64;
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                read += count as u64;
                hash.update(&buffer[..count]);
            }
            if read != length {
                return Err(io::Error::other("plugin file changed while it was hashed"));
            }
        } else {
            return Err(io::Error::other(format!(
                "unsupported plugin entry type: {}",
                path.display()
            )));
        }
    }
    Ok(format!("sha256:{}", hex::encode(hash.finalize())))
}

fn is_skipped(name: &str) -> bool {
    matches!(name, "__pycache__" | ".venv") || name.ends_with(".pyc") || name.ends_with(".egg-info")
}

fn portable(relative: &Path) -> String {
    relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}
