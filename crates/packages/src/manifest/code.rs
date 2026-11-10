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

/// What runs a package's code: a program speaking the guest protocol, or a
/// WebAssembly module speaking the guest ABI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageRuntime {
    Process(ProcessRuntime),
    Wasm(WasmRuntime),
}

impl PackageRuntime {
    pub fn kind(&self) -> RuntimeKind {
        match self {
            Self::Process(_) => RuntimeKind::Process,
            Self::Wasm(_) => RuntimeKind::Wasm,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeKind {
    Process,
    Wasm,
}

/// `runtime.type = "process"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessRuntime {
    /// The program and its arguments; a path in the package starts `./`.
    pub command: Vec<String>,
    /// Whether `command` starts with an interpreter outside the package.
    pub interpreter: bool,
    pub protocol: Option<String>,
}

/// `runtime.type = "wasm"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WasmRuntime {
    /// The module in the package (`./…`).
    pub module: String,
    pub limits: WasmRuntimeLimits,
}

/// What one instance of a WASM guest may use (`runtime.limits`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WasmRuntimeLimits {
    /// The fuel each export may burn (about one unit per instruction);
    /// unset, none is counted.
    pub fuel: Option<u64>,
    /// How long each export may run (`timeoutMs`); unset, the longest any
    /// call of the guest may take (the call deadline, or a longer tool
    /// `timeout`).
    pub timeout: Option<Duration>,
    /// The most linear memory the instance may have (`memoryBytes`).
    pub memory_bytes: u64,
}

impl Default for WasmRuntimeLimits {
    fn default() -> Self {
        Self {
            fuel: None,
            timeout: None,
            memory_bytes: DEFAULT_WASM_MEMORY_BYTES,
        }
    }
}

/// A WASM guest's memory unless its manifest says otherwise: 256 MiB.
pub const DEFAULT_WASM_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
/// The bounds of `memoryBytes`: one page, and all a 32-bit module can
/// address.
const WASM_MEMORY_BYTES: std::ops::RangeInclusive<u64> = 64 * 1024..=4 * 1024 * 1024 * 1024;
/// The longest `timeoutMs`: an hour, as a tool's `timeout`.
const MAXIMUM_WASM_TIMEOUT_MS: u64 = 3_600_000;

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
    let mut permissions = fields
        .object("permissions")
        .map(|permissions| self::permissions(permissions, fields.issues()))
        .unwrap_or_default();
    let config_schema = fields.object("configSchema").cloned();
    if matches!(runtime, Some(PackageRuntime::Wasm(_))) {
        if fields.raw("permissions").is_some() {
            fields.issue(
                "permissions",
                "a WASM guest reaches nothing outside its module (no files, network or \
                 environment), so it asks for nothing",
            );
        }
        permissions = PackagePermissions {
            network: false,
            workspace: WorkspaceAccess::None,
            env: Vec::new(),
        };
    }
    Some(PackageCode {
        runtime,
        contributes,
        permissions,
        config_schema,
    })
}

fn runtime(object: &Map<String, Value>, issues: &mut Vec<String>) -> Option<PackageRuntime> {
    let mut fields = Fields::new(object, "runtime.", issues);
    match fields.string("type").as_deref() {
        Some("process") => process_runtime(fields),
        Some("wasm") => wasm_runtime(fields),
        _ => {
            fields.issue(
                "type",
                "expected \"process\" or \"wasm\"; the runtime is ignored",
            );
            None
        }
    }
}

fn process_runtime(mut fields: Fields<'_>) -> Option<PackageRuntime> {
    let command = fields.strings("command");
    if command.is_empty() {
        fields.issue(
            "command",
            "a process runtime needs a command; the runtime is ignored",
        );
        return None;
    }
    Some(PackageRuntime::Process(ProcessRuntime {
        command,
        interpreter: fields.bool("interpreter").unwrap_or(false),
        protocol: fields.string("protocol"),
    }))
}

fn wasm_runtime(mut fields: Fields<'_>) -> Option<PackageRuntime> {
    let Some(module) = fields.string("module").filter(|module| !module.is_empty()) else {
        fields.issue(
            "module",
            "a wasm runtime needs a module (./….wasm); the runtime is ignored",
        );
        return None;
    };
    let limits = fields
        .object("limits")
        .map(|limits| {
            let mut fields = Fields::new(limits, "runtime.limits.", fields.issues());
            wasm_limits(&mut fields)
        })
        .unwrap_or_default();
    Some(PackageRuntime::Wasm(WasmRuntime { module, limits }))
}

/// Each limit on its own: one out of its bounds is an issue, and its
/// default applies.
fn wasm_limits(fields: &mut Fields<'_>) -> WasmRuntimeLimits {
    let mut whole = |key: &str, bounds: std::ops::RangeInclusive<u64>, fallback: &str| {
        let value = fields.raw(key)?;
        let number = value.as_u64().filter(|number| bounds.contains(number));
        if number.is_none() {
            fields.issue(
                key,
                &format!(
                    "expected a whole number from {} to {}; {fallback}",
                    bounds.start(),
                    bounds.end()
                ),
            );
        }
        number
    };
    let fuel = whole("fuel", 1..=u64::MAX, "no fuel is counted");
    let timeout = whole(
        "timeoutMs",
        1..=MAXIMUM_WASM_TIMEOUT_MS,
        "the guest's longest call bounds it",
    )
    .map(Duration::from_millis);
    let memory_bytes = whole("memoryBytes", WASM_MEMORY_BYTES, "256 MiB is used")
        .unwrap_or(DEFAULT_WASM_MEMORY_BYTES);
    WasmRuntimeLimits {
        fuel,
        timeout,
        memory_bytes,
    }
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
