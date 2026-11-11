//! Claude Code's agent files (`agents/*.md`) as roles
//! (site/content/docs/writing-plugins.en.mdx, "Claude Code agent files"):
//! the frontmatter's `name`, `description`, `tools`, `disallowedTools`,
//! `permissionMode` and `model`, and the body as the instructions. The
//! files are a third party's, so each key is read on its own: one of the
//! wrong type is left out with an issue. The keys that narrow never widen:
//! a Claude tool this Agent has no counterpart for is left out of `tools`,
//! a denied tool's specifier denies the whole tool, an unknown
//! `permissionMode` caps at manual, and a narrowing key that cannot be read
//! makes the agent unusable rather than allowing more. Other keys are
//! ignored, as the Skill catalog ignores a third party's.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::agent_settings::AgentPermissionMode;
use hanafubuki_agent_foundation::paths::ResolvedPath;
use serde_yaml::{Mapping, Value};

use crate::commands::split_frontmatter;

/// How deep below a declared directory agent files are looked for.
const MAX_DIRECTORY_DEPTH: usize = 8;
/// The `model` that keeps the model of the session or the parent role.
const INHERIT_MODEL: &str = "inherit";
/// What Plan mode removes from a default session: its built-in tools that
/// are not read-only (the facade's tests derive them from its plugins). An
/// agent in `permissionMode: plan` denies them. The kernel's own tools are
/// never a scope's to remove.
pub const PLAN_MODE_DENIED_TOOLS: [&str; 15] = [
    "append_agent_memory_note",
    "apply_patch",
    "create_goal",
    "delete_path",
    "disable_skill",
    "enable_skill",
    "exec_command",
    "followup_task",
    "interrupt_agent",
    "move_path",
    "resize_execution",
    "send_message",
    "spawn_agent",
    "update_goal",
    "write_stdin",
];
/// Every MCP server's tool, which `plan` denies unless `tools` lists them.
const MCP_TOOLS: &str = "mcp__*";

/// One agent file, read as a role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeAgent {
    pub file: PathBuf,
    /// The frontmatter's `name`, when it is text; the role's name, if it
    /// can be one, else the file's stem.
    pub name: Option<String>,
    pub description: Option<String>,
    /// The body, when it is not blank.
    pub instructions: Option<String>,
    /// The tools it allows, by this Agent's names; `None` without `tools`,
    /// which narrows nothing. A list whose every tool was left out is
    /// empty, and allows none.
    pub tools: Option<Vec<String>>,
    /// The tools it denies, by this Agent's names: those of
    /// `disallowedTools`, and the write tools under `permissionMode: plan`.
    pub denied_tools: Vec<String>,
    /// The widest permission mode it runs in, from `permissionMode`.
    pub permission_mode_cap: Option<AgentPermissionMode>,
    /// Why it cannot be used, when it cannot.
    pub unusable: Option<String>,
    /// What its reading left out.
    pub issues: Vec<String>,
}

/// The agent files of a package's `agents` component, by path, and what
/// could not be read as one at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClaudeAgentFiles {
    pub agents: Vec<ClaudeAgent>,
    pub issues: Vec<String>,
}

/// The agent files of `paths`: each a directory, whose `.md` files (in its
/// subdirectories too) are read by path, or one `.md` file.
pub fn read_claude_agents(paths: &[PathBuf]) -> ClaudeAgentFiles {
    let mut read = ClaudeAgentFiles::default();
    let mut files = Vec::new();
    for path in paths {
        match ResolvedPath::resolve(path) {
            Ok(path) if path.system().is_dir() => {
                collect_files(&path, 0, &mut files, &mut read.issues)
            }
            Ok(path) if is_markdown(path.system()) => files.push(path),
            Ok(path) => read.issues.push(format!(
                "{} is not a directory or a `.md` file; it is not read",
                path.native_text()
            )),
            Err(error) => read
                .issues
                .push(format!("{} cannot be read: {error}", path.display())),
        }
    }
    files.sort_by(|first, second| first.key().cmp(second.key()));
    files.dedup_by(|second, first| first.key() == second.key());
    for file in files {
        match read_agent(&file) {
            Ok(agent) => read.agents.push(agent),
            Err(issue) => read.issues.push(issue),
        }
    }
    read
}

/// The Agent's name for the Claude Code tool `name`: a fixed table, and an
/// MCP tool (`mcp__<server>__<tool>`) by its own name.
fn claude_tool_name(name: &str) -> Option<&str> {
    let ours = match name {
        "Read" => "read_file",
        "Grep" => "search_text",
        "Glob" | "LS" => "list_directory",
        "Edit" | "MultiEdit" | "Write" => "apply_patch",
        "Bash" => "exec_command",
        "WebFetch" => "fetch_web_page",
        "WebSearch" => "web_search",
        _ => {
            let (server, tool) = name.strip_prefix("mcp__")?.split_once("__")?;
            return (!server.is_empty() && !tool.is_empty()).then_some(name);
        }
    };
    Some(ours)
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
                "the agent directory {} cannot be read: {error}",
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
        } else if is_markdown(path.system()) {
            files.push(path);
        }
    }
}

fn is_markdown(path: &Path) -> bool {
    path.is_file() && path.extension().is_some_and(|ext| ext == "md")
}

/// The agent `file` defines; the error says why it defines none.
fn read_agent(file: &ResolvedPath) -> Result<ClaudeAgent, String> {
    let location = file.native_text();
    let text = fs::read_to_string(file.system())
        .map_err(|error| format!("{location} cannot be read: {error}"))?
        .replace("\r\n", "\n");
    let (frontmatter, body) = split_frontmatter(&text).ok_or_else(|| {
        format!("{location} has no frontmatter (a block between `---` lines); it is not read")
    })?;
    let fields = frontmatter_fields(frontmatter)
        .map_err(|why| format!("{location}: its frontmatter {why}; it is not read"))?;
    let body = body.trim();
    let mut agent = ClaudeAgent {
        file: file.native().to_path_buf(),
        name: None,
        description: None,
        instructions: (!body.is_empty()).then(|| body.to_string()),
        tools: None,
        denied_tools: Vec::new(),
        permission_mode_cap: None,
        unusable: None,
        issues: Vec::new(),
    };
    agent.name = text_field(
        &fields,
        "name",
        "the file's name is used",
        &mut agent.issues,
    );
    agent.description = text_field(&fields, "description", "it is ignored", &mut agent.issues);
    read_tools(&fields, &mut agent);
    read_disallowed_tools(&fields, &mut agent);
    read_permission_mode(&fields, &mut agent);
    agent.denied_tools.sort();
    agent.denied_tools.dedup();
    read_model(&fields, &mut agent.issues);
    Ok(agent)
}

/// The frontmatter's mapping. Hand-written frontmatter is often not quite
/// YAML (`description: Use it when: …`), so a plain value holding `: ` is
/// quoted and the text read again before it is given up on, as the Skill
/// catalog does.
fn frontmatter_fields(frontmatter: &str) -> Result<Mapping, String> {
    let parsed = serde_yaml::from_str::<Value>(frontmatter).or_else(|error| {
        quote_plain_values(frontmatter)
            .and_then(|repaired| serde_yaml::from_str(&repaired).ok())
            .ok_or_else(|| format!("is not YAML: {error}"))
    })?;
    match parsed {
        Value::Mapping(fields) => Ok(fields),
        Value::Null => Ok(Mapping::new()),
        _ => Err("is not a mapping of keys".to_string()),
    }
}

/// `frontmatter` with each top-level plain value that holds `: ` quoted;
/// `None` when there is none.
fn quote_plain_values(frontmatter: &str) -> Option<String> {
    let mut changed = false;
    let lines: Vec<String> = frontmatter
        .lines()
        .map(|line| {
            let quoted = line
                .split_once(": ")
                .filter(|(key, _)| !key.is_empty() && !key.starts_with([' ', '\t', '#', '-']))
                .map(|(key, value)| (key, value.trim()))
                .filter(|(_, value)| {
                    value.contains(": ") && !value.starts_with(['\'', '"', '|', '>', '[', '{'])
                })
                .map(|(key, value)| format!("{key}: '{}'", value.replace('\'', "''")));
            changed |= quoted.is_some();
            quoted.unwrap_or_else(|| line.to_string())
        })
        .collect();
    changed.then(|| lines.join("\n"))
}

/// The text of `key`, when it is text that is not blank.
fn text_field(
    fields: &Mapping,
    key: &str,
    otherwise: &str,
    issues: &mut Vec<String>,
) -> Option<String> {
    match fields.get(key)? {
        Value::Null => None,
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::String(_) => {
            issues.push(format!("`{key}` is empty; {otherwise}"));
            None
        }
        _ => {
            issues.push(format!("`{key}` is not text; {otherwise}"));
            None
        }
    }
}

/// `tools`: Claude's comma-separated names, or a list of them.
fn read_tools(fields: &Mapping, agent: &mut ClaudeAgent) {
    let Some(names) = tool_names(fields, "tools", "every tool", agent) else {
        return;
    };
    let mut tools = BTreeSet::new();
    for name in names {
        match claude_tool_name(&name) {
            Some(ours) => {
                tools.insert(ours.to_string());
            }
            None => agent.issues.push(format!(
                "the tool `{name}` has no counterpart in this Agent; it is left out"
            )),
        }
    }
    agent.tools = Some(tools.into_iter().collect());
}

/// `disallowedTools`: written as `tools` is. A tool with a specifier
/// (`Bash(git push:*)`) is denied whole, since a pattern here names tools,
/// not their arguments.
fn read_disallowed_tools(fields: &Mapping, agent: &mut ClaudeAgent) {
    let Some(names) = tool_names(fields, "disallowedTools", "the tools it denies", agent) else {
        return;
    };
    for name in names {
        let (base, specified) = match name.split_once('(') {
            Some((base, _)) => (base.trim(), true),
            None => (name.as_str(), false),
        };
        if let Some(server) = whole_mcp_server(base) {
            agent.denied_tools.push(format!("mcp__{server}__*"));
            continue;
        }
        let Some(ours) = claude_tool_name(base) else {
            agent.issues.push(format!(
                "the denied tool `{name}` has no counterpart in this Agent, so there is none \
                 to deny; it is left out"
            ));
            continue;
        };
        if specified {
            agent.issues.push(format!(
                "the denied tool `{name}` names some of a tool's calls, which a role cannot \
                 say; the whole `{ours}` is denied"
            ));
        }
        agent.denied_tools.push(ours.to_string());
    }
}

/// The server of a whole-server entry, `mcp__<server>` with no tool part,
/// which Claude Code reads as every tool of that server.
fn whole_mcp_server(name: &str) -> Option<&str> {
    let server = name.strip_prefix("mcp__")?;
    (!server.is_empty() && !server.contains("__")).then_some(server)
}

/// The names `key` lists, trimmed and not blank: a comma-separated string
/// or a list, whose item that is not text is left out with an issue.
/// `None` without the key, or when it is neither, which makes the agent
/// unusable, since leaving it out would allow `widening`.
fn tool_names(
    fields: &Mapping,
    key: &str,
    widening: &str,
    agent: &mut ClaudeAgent,
) -> Option<Vec<String>> {
    let names: Vec<String> = match fields.get(key)? {
        Value::Null => return None,
        Value::String(names) => names.split(',').map(str::to_string).collect(),
        Value::Sequence(items) => items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item.as_str() {
                Some(name) => Some(name.to_string()),
                None => {
                    agent
                        .issues
                        .push(format!("`{key}[{index}]` is not text; it is left out"));
                    None
                }
            })
            .collect(),
        _ => {
            unusable(
                agent,
                format!("`{key}` is not text or a list, and leaving it out would allow {widening}"),
            );
            return None;
        }
    };
    Some(
        names
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// `permissionMode`, as the cap of the mode the role runs in: Claude's
/// `default` asks, `acceptEdits` lets edits through, `bypassPermissions`
/// asks for nothing. `plan` is manual without the write tools; MCP tools
/// are among those unless `tools` lists them (then only those listed are
/// allowed already, and denying `mcp__*` would remove them). A mode a cap
/// cannot say, or one unknown, is manual.
fn read_permission_mode(fields: &Mapping, agent: &mut ClaudeAgent) {
    let mode = match fields.get("permissionMode") {
        None | Some(Value::Null) => return,
        Some(Value::String(mode)) => mode.trim(),
        Some(_) => {
            unusable(
                agent,
                "`permissionMode` is not text, and leaving it out would allow every permission \
                 mode"
                    .to_string(),
            );
            return;
        }
    };
    let cap = match mode {
        "default" => AgentPermissionMode::Manual,
        "acceptEdits" => AgentPermissionMode::AutoReview,
        "bypassPermissions" => AgentPermissionMode::FullAccess,
        "plan" => {
            agent
                .denied_tools
                .extend(PLAN_MODE_DENIED_TOOLS.map(str::to_string));
            if agent.tools.is_none() {
                agent.denied_tools.push(MCP_TOOLS.to_string());
            }
            AgentPermissionMode::Manual
        }
        "dontAsk" => {
            agent.issues.push(
                "`permissionMode` `dontAsk` denies what it does not allow, which a cap cannot \
                 say; the role runs in manual"
                    .to_string(),
            );
            AgentPermissionMode::Manual
        }
        other => {
            agent.issues.push(format!(
                "`permissionMode` `{other}` is not one of Claude Code's modes; the role runs in \
                 manual"
            ));
            AgentPermissionMode::Manual
        }
    };
    agent.permission_mode_cap = Some(cap);
}

/// Marks `agent` unusable for `why`, unless an earlier key did.
fn unusable(agent: &mut ClaudeAgent, why: String) {
    agent.unusable.get_or_insert(why);
}

/// `model`: `inherit` keeps the model of the session (or of a role it
/// extends); Claude's other values (`sonnet`, `opus`, `haiku`) name no
/// model of this Agent's.
fn read_model(fields: &Mapping, issues: &mut Vec<String>) {
    match fields.get("model") {
        None | Some(Value::Null) => {}
        Some(Value::String(model)) if model.trim() == INHERIT_MODEL => {}
        Some(Value::String(model)) => issues.push(format!(
            "`model` `{}` is ignored: Claude's models are not this Agent's, so the model \
             is inherited",
            model.trim()
        )),
        Some(_) => issues.push("`model` is not text; it is ignored".to_string()),
    }
}

#[cfg(test)]
mod tests;
