// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/command_migration.rs` and
// `codex-rs/core-plugins/src/command_migration/plugin.rs` (a package's
// `commands/*.md` turned into Skills at install: frontmatter with a
// description required, `source-command-` names, at most 4,000 bytes each),
// and the migrated-command directory of `codex-rs/utils/plugins/src/lib.rs`,
// at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: only the plugin migration is ported (the
// description is required, so there is no name fallback, and the plugin
// profile rewrites no terms); the command paths come from the parsed
// manifest.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_yaml::Value as YamlValue;

mod render;

use render::{slugify, yaml_string};

const COMMAND_SKILL_PREFIX: &str = "source-command";
const MAX_SKILL_NAME_CHARS: usize = 64;
const MAX_MIGRATED_SKILL_BYTES: usize = 4_000;
const DEFAULT_COMMANDS_DIR: &str = "commands";

/// The Skills a package's commands became, in the installed package.
pub fn migrated_command_skills_root(package_root: &Path) -> PathBuf {
    package_root
        .join(".codex-plugin")
        .join("migrated-command-skills")
}

/// Turns the commands of the package at `root` (the manifest's `commands`,
/// else `commands/`) into Skills under [`migrated_command_skills_root`],
/// replacing what an earlier migration wrote; returns their names.
pub(crate) fn migrate_commands(root: &Path, declared: &[PathBuf]) -> io::Result<Vec<String>> {
    let target = migrated_command_skills_root(root);
    if target.is_dir() {
        fs::remove_dir_all(&target)?;
    } else if target.exists() {
        fs::remove_file(&target)?;
    }
    let default = [root.join(DEFAULT_COMMANDS_DIR)];
    let paths = if declared.is_empty() {
        &default[..]
    } else {
        declared
    };
    let mut sources = Vec::new();
    for path in paths {
        sources.extend(supported_command_sources(path)?);
    }
    import(unique_command_sources(sources), &target)
}

#[derive(Debug)]
struct ParsedCommand {
    description: Option<String>,
    body: String,
}

#[derive(Debug, PartialEq, Eq)]
struct CommandSource {
    file: PathBuf,
    name: String,
    source_name: String,
}

fn import(sources: Vec<CommandSource>, target: &Path) -> io::Result<Vec<String>> {
    let mut imported = Vec::new();
    for source in sources {
        let command = parse_command(&fs::read_to_string(&source.file)?);
        let directory = target.join(&source.name);
        let Some(description) = command.description.as_deref() else {
            continue;
        };
        if directory.exists() {
            continue;
        }
        let rendered = render_skill(
            &command.body,
            &source.name,
            description,
            &source.source_name,
        );
        if rendered.len() > MAX_MIGRATED_SKILL_BYTES {
            continue;
        }
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("SKILL.md"), rendered)?;
        imported.push(source.name);
    }
    Ok(imported)
}

fn supported_command_sources(commands: &Path) -> io::Result<Vec<CommandSource>> {
    let mut sources = Vec::new();
    for file in command_files(commands)? {
        let command = parse_command(&fs::read_to_string(&file)?);
        let source_name = source_name(commands, &file);
        if let Some(name) = skill_name_if_supported(&source_name, &file, &command) {
            sources.push(CommandSource {
                file,
                name,
                source_name,
            });
        }
    }
    Ok(sources)
}

/// Commands whose Skill names collide are all skipped: neither can be
/// chosen over the other.
fn unique_command_sources(sources: Vec<CommandSource>) -> Vec<CommandSource> {
    let mut by_name = BTreeMap::<String, BTreeMap<PathBuf, String>>::new();
    for source in sources {
        by_name
            .entry(source.name)
            .or_default()
            .insert(source.file, source.source_name);
    }
    by_name
        .into_iter()
        .filter_map(|(name, files)| {
            let mut files = files.into_iter();
            let (file, source_name) = files.next()?;
            files.next().is_none().then_some(CommandSource {
                file,
                name,
                source_name,
            })
        })
        .collect()
}

fn command_files(commands: &Path) -> io::Result<Vec<PathBuf>> {
    if commands.is_file() {
        let markdown = commands.extension().and_then(|ext| ext.to_str()) == Some("md");
        return Ok(markdown
            .then(|| commands.to_path_buf())
            .into_iter()
            .collect());
    }
    let mut files = Vec::new();
    collect_markdown_files(commands, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_markdown_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_markdown_files(&path, files)?;
        } else if kind.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path);
        }
    }
    Ok(())
}

fn parse_command(content: &str) -> ParsedCommand {
    let whole = || ParsedCommand {
        description: None,
        body: content.to_string(),
    };
    let Some(rest) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return whole();
    };
    let Some((end, body_start)) = frontmatter_end(rest) else {
        return whole();
    };
    ParsedCommand {
        description: description(&rest[..end]),
        body: rest[body_start..].to_string(),
    }
}

fn frontmatter_end(rest: &str) -> Option<(usize, usize)> {
    [
        "\r\n---\r\n",
        "\r\n---\n",
        "\n---\r\n",
        "\n---\n",
        "\r\n---",
        "\n---",
    ]
    .into_iter()
    .filter_map(|delimiter| rest.find(delimiter).map(|end| (end, end + delimiter.len())))
    .min_by_key(|(end, _)| *end)
}

/// A non-empty `description` scalar of the frontmatter.
fn description(frontmatter: &str) -> Option<String> {
    let parsed: YamlValue = serde_yaml::from_str(frontmatter).ok()?;
    let value = parsed
        .as_mapping()?
        .iter()
        .find_map(|(key, value)| (key.as_str()?.trim() == "description").then_some(value))?;
    let description = match value {
        YamlValue::String(value) => value.trim().to_string(),
        YamlValue::Bool(value) => value.to_string(),
        YamlValue::Number(value) => value.to_string(),
        _ => return None,
    };
    (!description.is_empty()).then_some(description)
}

fn skill_name_if_supported(
    source_name: &str,
    file: &Path,
    command: &ParsedCommand,
) -> Option<String> {
    if file.file_stem().and_then(|stem| stem.to_str()) == Some("README") {
        return None;
    }
    command.description.as_ref()?;
    let name = slugify(&format!("{COMMAND_SKILL_PREFIX}-{source_name}"));
    if name.chars().count() > MAX_SKILL_NAME_CHARS || has_runtime_expansion(&command.body) {
        return None;
    }
    Some(name)
}

fn source_name(commands: &Path, file: &Path) -> String {
    if commands.is_file() {
        return file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
    }
    file.strip_prefix(commands)
        .unwrap_or(file)
        .with_extension("")
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("-")
}

fn render_skill(body: &str, name: &str, description: &str, source_name: &str) -> String {
    let body = body.trim();
    let template = if body.is_empty() {
        "No command template body was found."
    } else {
        body
    };
    format!(
        "---\nname: {}\ndescription: {}\n---\n\n# {name}\n\nUse this skill when the user asks to run the migrated source command `{source_name}`.\n\n## Command Template\n\n{template}\n",
        yaml_string(name),
        yaml_string(description),
    )
}

/// Arguments, file references and shell expansion, which only the source
/// tool's command runner provides.
fn has_runtime_expansion(template: &str) -> bool {
    template.contains("$ARGUMENTS")
        || template
            .as_bytes()
            .windows(2)
            .any(|window| window[0] == b'$' && window[1].is_ascii_digit())
        || (template.contains("{{") && template.contains("}}"))
        || template.contains("!`")
        || template.contains("! `")
        || template
            .split_whitespace()
            .any(|token| token.strip_prefix('@').is_some_and(|rest| !rest.is_empty()))
}

#[cfg(test)]
mod tests;
