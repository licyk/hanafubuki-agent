//! The blocks only this project's manifest (`.hanafubuki-plugin/plugin.json`)
//! has: what runs (`runtime`), what it contributes, what it may reach
//! (`permissions`) and its settings (`configSchema`). The guest host reads
//! them; the store keeps them. Each is read on its own: a block that cannot
//! be used is dropped with an issue, and so is one entry of a list.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::fields::{kind, Fields};

#[derive(Clone, Debug, PartialEq)]
pub struct PackageCode {
    pub runtime: Option<PackageRuntime>,
    pub contributes: PackageContributes,
    pub permissions: PackagePermissions,
    /// The guest's settings, a JSON Schema.
    pub config_schema: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageRuntime {
    pub kind: RuntimeKind,
    /// The program and its arguments; a path in the package starts `./`.
    pub command: Vec<String>,
    /// Whether `command` starts with an interpreter outside the package.
    pub interpreter: bool,
    pub protocol: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeKind {
    Process,
    Wasm,
}

/// What the guest contributes, declared up front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackageContributes {
    pub tools: Vec<ContributedTool>,
    pub context_sections: Vec<ContributedSection>,
    /// The prompt slot the guest fills, when it contributes a prompt.
    pub prompt_slot: Option<String>,
    pub tool_lifecycle_hooks: bool,
    pub continuation_hooks: bool,
    pub observers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContributedTool {
    pub name: String,
    pub description: String,
    pub input_schema: Map<String, Value>,
    /// How long one call may run (`timeout`, in seconds); unset takes the
    /// guest host's call deadline.
    pub timeout: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContributedSection {
    pub kind: String,
    pub renderer: Option<String>,
}

/// What a package's code may reach: what its manifest asks for, and what
/// the user granted, which the install record keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PackagePermissions {
    pub network: bool,
    pub workspace: WorkspaceAccess,
    /// The environment variables the user maps at enable time.
    pub env: Vec<String>,
}

/// Ordered by what it allows: `None < Read < Write`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceAccess {
    None,
    #[default]
    Read,
    Write,
}

/// Reads one entry of a list, labelled for its issues.
type EntryReader<T> = fn(&Map<String, Value>, &str, &mut Vec<String>) -> Option<T>;

const CONTRIBUTES: &[&str] = &[
    "tools",
    "contextSections",
    "prompt",
    "toolLifecycleHooks",
    "continuationHooks",
    "observers",
];

/// The code blocks of `object`, or `None` when it has none.
pub(super) fn parse(object: &Map<String, Value>, issues: &mut Vec<String>) -> Option<PackageCode> {
    let mut fields = Fields::new(object, "", issues);
    if ["runtime", "contributes", "permissions", "configSchema"]
        .iter()
        .all(|key| fields.raw(key).is_none())
    {
        return None;
    }
    let runtime = fields
        .object("runtime")
        .and_then(|runtime| self::runtime(runtime, fields.issues()));
    let contributes = fields
        .object("contributes")
        .map(|contributes| self::contributes(contributes, fields.issues()))
        .unwrap_or_default();
    let permissions = fields
        .object("permissions")
        .map(|permissions| self::permissions(permissions, fields.issues()))
        .unwrap_or_default();
    let config_schema = fields.object("configSchema").cloned();
    Some(PackageCode {
        runtime,
        contributes,
        permissions,
        config_schema,
    })
}

fn runtime(object: &Map<String, Value>, issues: &mut Vec<String>) -> Option<PackageRuntime> {
    let mut fields = Fields::new(object, "runtime.", issues);
    let kind = match fields.string("type").as_deref() {
        Some("process") => RuntimeKind::Process,
        Some("wasm") => RuntimeKind::Wasm,
        _ => {
            fields.issue(
                "type",
                "expected \"process\" or \"wasm\"; the runtime is ignored",
            );
            return None;
        }
    };
    let command = fields.strings("command");
    if command.is_empty() {
        fields.issue(
            "command",
            "a runtime needs a command; the runtime is ignored",
        );
        return None;
    }
    Some(PackageRuntime {
        kind,
        command,
        interpreter: fields.bool("interpreter").unwrap_or(false),
        protocol: fields.string("protocol"),
    })
}

fn contributes(object: &Map<String, Value>, issues: &mut Vec<String>) -> PackageContributes {
    let mut fields = Fields::new(object, "contributes.", issues);
    for key in object
        .keys()
        .filter(|key| !CONTRIBUTES.contains(&key.as_str()))
    {
        fields.issue(key, "not a contribution this version knows");
    }
    let tools = list(&mut fields, "tools", tool);
    let context_sections = list(&mut fields, "contextSections", section);
    let prompt_slot = fields.object("prompt").and_then(|prompt| {
        Fields::new(prompt, "contributes.prompt.", fields.issues()).string("slot")
    });
    PackageContributes {
        tools,
        context_sections,
        prompt_slot,
        tool_lifecycle_hooks: fields.bool("toolLifecycleHooks").unwrap_or(false),
        continuation_hooks: fields.bool("continuationHooks").unwrap_or(false),
        observers: fields.strings("observers"),
    }
}

/// The objects of the list at `key`, each read by `read`; an entry that
/// cannot be one is dropped alone.
fn list<T>(fields: &mut Fields<'_>, key: &str, read: EntryReader<T>) -> Vec<T> {
    let Some(raw) = fields.raw(key) else {
        return Vec::new();
    };
    let Value::Array(items) = raw else {
        fields.issue(key, &format!("expected a list, found {}", kind(raw)));
        return Vec::new();
    };
    let mut entries = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let label = format!("{}[{index}]", fields.label(key));
        match item {
            Value::Object(object) => entries.extend(read(object, &label, fields.issues())),
            other => fields.issues().push(format!(
                "ignoring {label}: expected an object, found {}",
                kind(other)
            )),
        }
    }
    entries
}

fn tool(
    object: &Map<String, Value>,
    label: &str,
    issues: &mut Vec<String>,
) -> Option<ContributedTool> {
    let mut fields = Fields::new(object, &format!("{label}."), issues);
    let name = fields.string("name").filter(|name| !name.is_empty());
    let input_schema = fields.object("inputSchema").cloned();
    let (Some(name), Some(input_schema)) = (name, input_schema) else {
        fields.issues().push(format!(
            "ignoring {label}: a tool needs a name and an inputSchema object"
        ));
        return None;
    };
    let timeout = match fields.raw("timeout") {
        None | Some(Value::Null) => None,
        Some(value) => match value
            .as_f64()
            .filter(|seconds| *seconds > 0.0 && *seconds <= MAXIMUM_TOOL_TIMEOUT_SECONDS)
        {
            Some(seconds) => Some(Duration::from_secs_f64(seconds)),
            None => {
                fields.issue(
                    "timeout",
                    "expected a number of seconds above 0 and at most 3600; the call deadline is used",
                );
                None
            }
        },
    };
    Some(ContributedTool {
        name,
        description: fields.string("description").unwrap_or_default(),
        input_schema,
        timeout,
    })
}

/// The longest a manifest may let one tool call run.
const MAXIMUM_TOOL_TIMEOUT_SECONDS: f64 = 3600.0;

fn section(
    object: &Map<String, Value>,
    label: &str,
    issues: &mut Vec<String>,
) -> Option<ContributedSection> {
    let mut fields = Fields::new(object, &format!("{label}."), issues);
    let Some(kind) = fields.string("kind").filter(|kind| !kind.is_empty()) else {
        fields
            .issues()
            .push(format!("ignoring {label}: a context section needs a kind"));
        return None;
    };
    Some(ContributedSection {
        kind,
        renderer: fields.string("renderer"),
    })
}

fn permissions(object: &Map<String, Value>, issues: &mut Vec<String>) -> PackagePermissions {
    let mut fields = Fields::new(object, "permissions.", issues);
    let workspace = match fields.string("workspace").as_deref() {
        None | Some("read") => WorkspaceAccess::Read,
        Some("none") => WorkspaceAccess::None,
        Some("write") => WorkspaceAccess::Write,
        Some(_) => {
            fields.issue(
                "workspace",
                "expected \"none\", \"read\" or \"write\"; read is used",
            );
            WorkspaceAccess::Read
        }
    };
    PackagePermissions {
        network: fields.bool("network").unwrap_or(false),
        workspace,
        env: fields.strings("env"),
    }
}
