//! Where role definitions are found: a `config.toml`'s `[agents]` entries,
//! each with the role file its `configFile` names, and the role files of an
//! `agents/` directory, by path.

use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};
use serde_json::{Map, Value};

use super::decode::decode_role;
use super::{is_role_name, RoleDefinition, RoleFiles, RoleOrigin, RoleSource};
use crate::file::{ConfigFile, FileAccess, FileRead};

const CONFIG_FILE: &str = "configFile";
/// Codex's name for `configFile`.
const CODEX_CONFIG_FILE: &str = "config_file";
const NAME: &str = "name";
/// How deep below `agents/` role files are looked for.
const MAX_DIRECTORY_DEPTH: usize = 8;

/// The document `file` holds; empty when there is none.
pub(super) fn read_document(file: &Path) -> Result<Map<String, Value>, String> {
    match ConfigFile::new(file.to_path_buf(), FileAccess::Shared).read()? {
        FileRead::Missing => Ok(Map::new()),
        FileRead::Document(document) => Ok(document),
        FileRead::Unusable(reason) => Err(format!("{}: {reason}", file.display())),
    }
}

/// Adds the entries of `agents`, the table at `key` of `file`, to `source`,
/// a `configFile` read relative to `folder`; returns the role files they
/// name, which the directory then leaves to them.
pub(super) fn read_entries(
    file: &Path,
    key: &str,
    agents: Option<&Value>,
    folder: &Path,
    source: &mut RoleSource,
) -> Vec<PathKey> {
    let Some(agents) = agents else {
        return Vec::new();
    };
    let Some(agents) = agents.as_object() else {
        source.issues.push(format!(
            "[{key}] in {} is not a table of roles; it is ignored",
            file.display()
        ));
        return Vec::new();
    };
    let mut declared = Vec::new();
    for (name, entry) in agents {
        let entry_key = format!("{key}.{name}");
        let Some(entry) = entry.as_object() else {
            source.issues.push(format!(
                "{entry_key} in {} is not a role's table; it is ignored (Codex's agent \
                 settings are not read here)",
                file.display()
            ));
            continue;
        };
        if !is_role_name(name) {
            source.issues.push(invalid_name(&format!(
                "[{entry_key}] in {}",
                file.display()
            )));
            continue;
        }
        let (definition, config_file) = read_entry(file, entry_key, name, entry, folder);
        declared.extend(config_file);
        source.definitions.push(definition);
    }
    declared
}

/// One `[agents.<name>]` entry: its keys read over those of the role file
/// its `configFile` names, if any.
fn read_entry(
    file: &Path,
    key: String,
    name: &str,
    entry: &Map<String, Value>,
    folder: &Path,
) -> (RoleDefinition, Option<PathKey>) {
    let config_file = entry
        .get(CONFIG_FILE)
        .or_else(|| entry.get(CODEX_CONFIG_FILE));
    let mut origin = RoleOrigin::Table {
        file: file.to_path_buf(),
        key,
        config_file: None,
    };
    let Some(config_file) = config_file else {
        return (decode_role(name.to_string(), origin, entry), None);
    };
    let Some(relative) = config_file.as_str().filter(|path| !path.is_empty()) else {
        let mut definition = RoleDefinition::empty(name.to_string(), origin);
        definition.unusable = Some(format!("`{CONFIG_FILE}` is not a path"));
        return (definition, None);
    };
    let requested = folder.join(relative);
    let resolved = match ResolvedPath::resolve(&requested) {
        Ok(resolved) => resolved,
        Err(error) => {
            if let RoleOrigin::Table { config_file, .. } = &mut origin {
                *config_file = Some(requested.clone());
            }
            let mut definition = RoleDefinition::empty(name.to_string(), origin);
            definition.unusable = Some(format!(
                "its {CONFIG_FILE} {} cannot be read: {error}",
                requested.display()
            ));
            return (definition, None);
        }
    };
    if let RoleOrigin::Table { config_file, .. } = &mut origin {
        *config_file = Some(resolved.native().to_path_buf());
    }
    let mut table = match read_role_file(&resolved) {
        Ok(table) => table,
        Err(reason) => {
            let mut definition = RoleDefinition::empty(name.to_string(), origin);
            definition.unusable = Some(format!("its {CONFIG_FILE} {reason}"));
            return (definition, Some(resolved.key().clone()));
        }
    };
    let file_name = table.remove(NAME);
    table.extend(entry.clone());
    let mut definition = decode_role(name.to_string(), origin, &table);
    if file_name
        .as_ref()
        .and_then(Value::as_str)
        .is_some_and(|written| written != name)
    {
        definition.issues.push(format!(
            "the `{NAME}` its {CONFIG_FILE} writes is not the entry's; the entry's `{name}` is used"
        ));
    }
    (definition, Some(resolved.key().clone()))
}

/// Adds the role files of `directory` (and of its subdirectories), by
/// path, to `source`: each `.toml` file but those `declared` by an entry.
pub(super) fn read_directory(directory: &Path, declared: &[PathKey], source: &mut RoleSource) {
    let Ok(root) = ResolvedPath::resolve(directory) else {
        return;
    };
    let mut files = Vec::new();
    collect_files(&root, 0, &mut files, &mut source.issues);
    files.retain(|file| !declared.contains(file.key()));
    read_definitions(files, &mut source.definitions, &mut source.issues);
}

/// The role files of `paths`: each a directory, read as an `agents/` is,
/// or one role file.
pub(super) fn read_paths(paths: &[PathBuf]) -> RoleFiles {
    let mut read = RoleFiles::default();
    let mut files = Vec::new();
    for path in paths {
        match ResolvedPath::resolve(path) {
            Ok(path) if path.system().is_dir() => {
                collect_files(&path, 0, &mut files, &mut read.issues)
            }
            Ok(path) => files.push(path),
            Err(error) => read
                .issues
                .push(format!("{} cannot be read: {error}", path.display())),
        }
    }
    read_definitions(files, &mut read.definitions, &mut read.issues);
    read
}

/// The definitions of `files`, by path, each read once.
fn read_definitions(
    mut files: Vec<ResolvedPath>,
    definitions: &mut Vec<RoleDefinition>,
    issues: &mut Vec<String>,
) {
    files.sort_by(|first, second| first.key().cmp(second.key()));
    files.dedup_by(|second, first| first.key() == second.key());
    for file in files {
        match read_file_definition(&file) {
            Ok(definition) => definitions.push(definition),
            Err(issue) => issues.push(issue),
        }
    }
}

fn collect_files(
    directory: &ResolvedPath,
    depth: usize,
    files: &mut Vec<ResolvedPath>,
    issues: &mut Vec<String>,
) {
    let entries = match fs::read_dir(directory.system()) {
        Ok(entries) => entries,
        Err(error) => {
            issues.push(format!(
                "the role directory {} cannot be read: {error}",
                directory.native_text()
            ));
            return;
        }
    };
    for entry in entries.flatten() {
        let Ok(path) = ResolvedPath::resolve_child(directory, &entry.file_name()) else {
            continue;
        };
        if path.system().is_dir() {
            if depth < MAX_DIRECTORY_DEPTH {
                collect_files(&path, depth + 1, files, issues);
            }
        } else if path.system().extension().is_some_and(|ext| ext == "toml") {
            files.push(path);
        }
    }
}

/// The role `file` defines, named by its `name` or else its stem; one that
/// cannot be read is kept as an unusable role of its stem, so it still
/// replaces a role of that name below it.
fn read_file_definition(file: &ResolvedPath) -> Result<RoleDefinition, String> {
    let origin = RoleOrigin::File {
        path: file.native().to_path_buf(),
    };
    let stem = file
        .native()
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| is_role_name(stem));
    let table = match read_role_file(file) {
        Ok(table) => table,
        Err(reason) => {
            let stem = stem.ok_or_else(|| invalid_name(file.native_text()))?;
            let mut definition = RoleDefinition::empty(stem.to_string(), origin);
            definition.unusable = Some(format!("the file {reason}"));
            return Ok(definition);
        }
    };
    let mut issues = Vec::new();
    let named = match table.get(NAME) {
        None => None,
        Some(Value::String(name)) if is_role_name(name) => Some(name.clone()),
        Some(_) => {
            issues.push(format!(
                "`{NAME}` is not a role name; the file's stem is used"
            ));
            None
        }
    };
    let name = named
        .or_else(|| stem.map(str::to_string))
        .ok_or_else(|| invalid_name(file.native_text()))?;
    let mut definition = decode_role(name, origin, &table);
    definition.issues.splice(0..0, issues);
    Ok(definition)
}

/// The table a role file holds; the error completes "the file …".
fn read_role_file(file: &ResolvedPath) -> Result<Map<String, Value>, String> {
    // The file is read by its native form, which its errors name.
    let path: PathBuf = file.native().to_path_buf();
    match ConfigFile::new(path, FileAccess::Shared).read()? {
        FileRead::Document(table) => Ok(table),
        FileRead::Missing => Err(format!("{} is missing", file.native_text())),
        FileRead::Unusable(reason) => Err(format!("{} is unusable: {reason}", file.native_text())),
    }
}

fn invalid_name(location: &str) -> String {
    format!(
        "{location} has no role name: a name holds 1 to {} bytes without spaces, control \
         characters or `:`; it is ignored",
        super::MAX_ROLE_NAME_BYTES
    )
}
