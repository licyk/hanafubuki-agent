//! The MCP server form and the server it describes: the transport (a
//! program, or a Streamable HTTP address), its environment or headers each
//! public or secret, and the common fields, the rarely changed ones under
//! "Advanced". A new server starts from the Agent's defaults for one.
//!
//! A secret row's value is a credential: it keeps the key the server
//! already references for that name, and a new one gets
//! `McpCredentialKey::for_server_secret`. Only a value that changed is
//! written; a key the server stops referencing is offered for removal.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hanafubuki_agent::{
    McpApplicationSnapshot, McpApprovalPolicy, McpCredentialKey, McpExposurePolicy, McpOAuthPolicy,
    McpServerConfig, McpServerId, McpStdioConfig, McpStreamableHttpConfig, McpTransportConfig,
};

use crate::bottom_pane::{FieldId, FormItem, FormView, RowValue};

pub(super) const ID: FieldId = "id";
pub(super) const TRANSPORT: FieldId = "transport";
pub(super) const PROGRAM: FieldId = "program";
pub(super) const ARGUMENTS: FieldId = "arguments";
pub(super) const CWD: FieldId = "cwd";
pub(super) const ENVIRONMENT: FieldId = "environment";
pub(super) const URL: FieldId = "url";
pub(super) const OAUTH: FieldId = "oauth";
pub(super) const HEADERS: FieldId = "headers";
pub(super) const ENABLED: FieldId = "enabled";
pub(super) const REQUIRED: FieldId = "required";
pub(super) const APPROVAL: FieldId = "approval";
pub(super) const EXPOSURE: FieldId = "exposure";
pub(super) const ENABLED_TOOLS: FieldId = "enabled_tools";
pub(super) const DISABLED_TOOLS: FieldId = "disabled_tools";
pub(super) const SAVE: FieldId = "save";

const STDIO: usize = 0;
const HTTP: usize = 1;
const OAUTH_POLICIES: [McpOAuthPolicy; 3] = [
    McpOAuthPolicy::Disabled,
    McpOAuthPolicy::Optional,
    McpOAuthPolicy::Required,
];
const APPROVALS: [McpApprovalPolicy; 3] = [
    McpApprovalPolicy::Allow,
    McpApprovalPolicy::Ask,
    McpApprovalPolicy::Deny,
];
const EXPOSURES: [McpExposurePolicy; 4] = [
    McpExposurePolicy::Direct,
    McpExposurePolicy::DirectModelOnly,
    McpExposurePolicy::Deferred,
    McpExposurePolicy::Hidden,
];
/// The timeouts and the call limit: their field, label and value.
const NUMBERS: [(FieldId, &str); 6] = [
    ("connect_timeout", "Connect timeout (s)"),
    ("initialize_timeout", "Initialize timeout (s)"),
    ("list_timeout", "List timeout (s)"),
    ("call_timeout", "Call timeout (s)"),
    ("shutdown_timeout", "Shutdown timeout (s)"),
    ("maximum_calls", "Concurrent calls"),
];

/// What saving the form writes.
pub(super) struct ServerWrite {
    pub(super) server: McpServerConfig,
    pub(super) secrets: Vec<(McpCredentialKey, String)>,
    pub(super) dropped: Vec<McpCredentialKey>,
}

/// The form over `existing`, or one for a new server from the defaults.
pub(super) fn server_form(
    existing: Option<&McpServerConfig>,
    snapshot: &McpApplicationSnapshot,
) -> FormView {
    let server = existing.cloned().unwrap_or_else(|| new_server(snapshot));
    let credential = |key: &McpCredentialKey| {
        snapshot
            .settings
            .credentials
            .get(key)
            .map(|credential| credential.value.clone())
            .unwrap_or_default()
    };
    let (stdio, http) = match &server.transport {
        McpTransportConfig::Stdio(stdio) => (Some(stdio), None),
        McpTransportConfig::StreamableHttp(http) => (None, Some(http)),
    };
    let environment = stdio.map_or_else(Vec::new, |stdio| {
        rows(
            &stdio.public_environment,
            &stdio.credential_environment,
            credential,
        )
    });
    let headers = http.map_or_else(Vec::new, |http| {
        rows(&http.public_headers, &http.credential_headers, credential)
    });
    let numbers = [
        server.connect_timeout_seconds,
        server.initialize_timeout_seconds,
        server.list_timeout_seconds,
        server.call_timeout_seconds,
        server.shutdown_timeout_seconds,
        server.maximum_concurrent_calls as u64,
    ];
    let mut items = Vec::new();
    if existing.is_none() {
        items.push(FormItem::text(ID, "Id", "").placeholder("my-server"));
    }
    items.extend([
        FormItem::choice(
            TRANSPORT,
            "Transport",
            &["stdio", "streamable_http"],
            if http.is_some() { HTTP } else { STDIO },
        ),
        FormItem::text(PROGRAM, "Program", stdio.map_or("", |s| &s.program))
            .placeholder("npx")
            .shown_if(TRANSPORT, STDIO),
        FormItem::text(
            ARGUMENTS,
            "Arguments",
            &stdio.map_or_else(String::new, |s| join_arguments(&s.argv)),
        )
        .placeholder("-y @scope/server")
        .shown_if(TRANSPORT, STDIO),
        FormItem::text(
            CWD,
            "Working directory",
            &stdio
                .and_then(|s| s.cwd.as_ref())
                .map_or_else(String::new, |cwd| cwd.display().to_string()),
        )
        .placeholder("the Agent's")
        .shown_if(TRANSPORT, STDIO),
        FormItem::rows(ENVIRONMENT, "Environment", environment, "Add a variable")
            .with_secret_column()
            .shown_if(TRANSPORT, STDIO),
        FormItem::text(URL, "URL", http.map_or("", |h| &h.url))
            .placeholder("https://example.com/mcp")
            .shown_if(TRANSPORT, HTTP),
        FormItem::choice(
            OAUTH,
            "OAuth",
            &["disabled", "optional", "required"],
            index_of(
                &OAUTH_POLICIES,
                http.map_or(snapshot.server_defaults.oauth, |h| h.oauth),
            ),
        )
        .shown_if(TRANSPORT, HTTP),
        FormItem::rows(HEADERS, "Headers", headers, "Add a header")
            .with_secret_column()
            .shown_if(TRANSPORT, HTTP),
        FormItem::toggle(ENABLED, "Enabled", server.enabled),
        FormItem::toggle(REQUIRED, "Required", server.required),
        FormItem::choice(
            APPROVAL,
            "Approval",
            &["allow", "ask", "deny"],
            index_of(&APPROVALS, server.approval_policy),
        )
        .advanced(),
        FormItem::choice(
            EXPOSURE,
            "Exposure",
            &["direct", "direct_model_only", "deferred", "hidden"],
            index_of(&EXPOSURES, server.exposure_policy),
        )
        .advanced(),
        FormItem::text(
            ENABLED_TOOLS,
            "Enabled tools",
            &join_tools(&server.enabled_tools),
        )
        .placeholder("all")
        .advanced(),
        FormItem::text(
            DISABLED_TOOLS,
            "Disabled tools",
            &join_tools(&server.disabled_tools),
        )
        .placeholder("none")
        .advanced(),
    ]);
    for ((id, label), value) in NUMBERS.into_iter().zip(numbers) {
        items.push(FormItem::text(id, label, &value.to_string()).advanced());
    }
    items.push(FormItem::action(SAVE, "Save"));
    let title = match existing {
        Some(server) => format!("Edit MCP server {}", server.id.as_str()),
        None => "Add an MCP server".to_string(),
    };
    FormView::new(title, items)
}

/// What saving writes; `None`, with the form saying why, when a field the
/// write needs is missing or not a number.
pub(super) fn server_write(
    form: &mut FormView,
    existing: Option<&McpServerConfig>,
    snapshot: &McpApplicationSnapshot,
) -> Option<ServerWrite> {
    form.clear_errors();
    let mut server = existing.cloned().unwrap_or_else(|| new_server(snapshot));
    let mut valid = true;
    if existing.is_none() {
        match McpServerId::parse(form.text(ID).trim()) {
            Ok(id) => server.id = id,
            Err(_) => {
                form.set_error(ID, "Use 1 to 128 letters, digits, `-`, `_` or `.`.");
                valid = false;
            }
        }
    }
    let mut numbers = [0u64; NUMBERS.len()];
    for (number, (id, _)) in numbers.iter_mut().zip(NUMBERS).rev() {
        match form.text(id).trim().parse() {
            Ok(value) => *number = value,
            Err(_) => {
                form.set_error(id, "Enter a whole number.");
                valid = false;
            }
        }
    }
    let previous = existing.map(credential_names).unwrap_or_default();
    let stored = |key: &McpCredentialKey| {
        snapshot
            .settings
            .credentials
            .get(key)
            .map(|credential| credential.value.as_str())
    };
    let mut secrets = Vec::new();
    let mut split = |rows: Vec<RowValue>| {
        let mut public = BTreeMap::new();
        let mut credentials = BTreeMap::new();
        for row in rows {
            if !row.secret {
                public.insert(row.name, row.value);
                continue;
            }
            let key = previous
                .get(&row.name)
                .cloned()
                .unwrap_or_else(|| McpCredentialKey::for_server_secret(&server.id, &row.name));
            if stored(&key) != Some(row.value.as_str()) {
                secrets.push((key.clone(), row.value));
            }
            credentials.insert(row.name, key);
        }
        (public, credentials)
    };
    server.transport = if form.choice(TRANSPORT) == HTTP {
        let url = form.text(URL).trim().to_string();
        if url.is_empty() {
            form.set_error(URL, "Enter the server's URL.");
            valid = false;
        }
        let (public_headers, credential_headers) = split(form.rows(HEADERS));
        McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
            url,
            public_headers,
            credential_headers,
            oauth: OAUTH_POLICIES[form.choice(OAUTH).min(OAUTH_POLICIES.len() - 1)],
        })
    } else {
        let program = form.text(PROGRAM).trim().to_string();
        if program.is_empty() {
            form.set_error(PROGRAM, "Enter the program that runs the server.");
            valid = false;
        }
        let cwd = form.text(CWD).trim();
        let (public_environment, credential_environment) = split(form.rows(ENVIRONMENT));
        McpTransportConfig::Stdio(McpStdioConfig {
            program,
            argv: split_arguments(form.text(ARGUMENTS)),
            cwd: (!cwd.is_empty()).then(|| PathBuf::from(cwd)),
            public_environment,
            credential_environment,
        })
    };
    if !valid {
        return None;
    }
    let [connect, initialize, list, call, shutdown, maximum_calls] = numbers;
    server.connect_timeout_seconds = connect;
    server.initialize_timeout_seconds = initialize;
    server.list_timeout_seconds = list;
    server.call_timeout_seconds = call;
    server.shutdown_timeout_seconds = shutdown;
    server.maximum_concurrent_calls = usize::try_from(maximum_calls).unwrap_or(usize::MAX);
    server.enabled = form.toggle(ENABLED);
    server.required = form.toggle(REQUIRED);
    server.approval_policy = APPROVALS[form.choice(APPROVAL).min(APPROVALS.len() - 1)];
    server.exposure_policy = EXPOSURES[form.choice(EXPOSURE).min(EXPOSURES.len() - 1)];
    server.enabled_tools = split_tools(form.text(ENABLED_TOOLS));
    server.disabled_tools = split_tools(form.text(DISABLED_TOOLS));
    let kept = credential_names(&server);
    let dropped = previous
        .into_values()
        .filter(|key| !kept.values().any(|kept| kept == key))
        .collect();
    Some(ServerWrite {
        server,
        secrets,
        dropped,
    })
}

/// The keys `server` references, by the variable or header they fill.
pub(super) fn credential_names(server: &McpServerConfig) -> BTreeMap<String, McpCredentialKey> {
    match &server.transport {
        McpTransportConfig::Stdio(stdio) => stdio.credential_environment.clone(),
        McpTransportConfig::StreamableHttp(http) => http.credential_headers.clone(),
    }
}

/// A server with the Agent's defaults and no transport fields yet.
fn new_server(snapshot: &McpApplicationSnapshot) -> McpServerConfig {
    let defaults = snapshot.server_defaults;
    McpServerConfig {
        // Replaced by the form's id before anything is written.
        id: McpServerId::parse("new").expect("`new` is a server id"),
        enabled: defaults.enabled,
        required: defaults.required,
        protocol_mode: defaults.protocol_mode,
        transport: McpTransportConfig::Stdio(McpStdioConfig {
            program: String::new(),
            argv: Vec::new(),
            cwd: None,
            public_environment: BTreeMap::new(),
            credential_environment: BTreeMap::new(),
        }),
        connect_timeout_seconds: defaults.connect_timeout_seconds,
        initialize_timeout_seconds: defaults.initialize_timeout_seconds,
        list_timeout_seconds: defaults.list_timeout_seconds,
        call_timeout_seconds: defaults.call_timeout_seconds,
        shutdown_timeout_seconds: defaults.shutdown_timeout_seconds,
        maximum_concurrent_calls: defaults.maximum_concurrent_calls,
        tool_maximum_concurrent_calls: BTreeMap::new(),
        enabled_tools: BTreeSet::new(),
        disabled_tools: BTreeSet::new(),
        approval_policy: defaults.approval_policy,
        tool_approval_policies: BTreeMap::new(),
        exposure_policy: defaults.exposure_policy,
        tool_exposure_policies: BTreeMap::new(),
    }
}

/// The public rows and the secret ones, by name, each secret with the
/// value its credential holds.
fn rows(
    public: &BTreeMap<String, String>,
    credentials: &BTreeMap<String, McpCredentialKey>,
    credential: impl Fn(&McpCredentialKey) -> String,
) -> Vec<RowValue> {
    let mut rows: Vec<RowValue> = public
        .iter()
        .map(|(name, value)| RowValue {
            name: name.clone(),
            value: value.clone(),
            secret: false,
        })
        .chain(credentials.iter().map(|(name, key)| RowValue {
            name: name.clone(),
            value: credential(key),
            secret: true,
        }))
        .collect();
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    rows
}

fn index_of<T: PartialEq>(options: &[T], value: T) -> usize {
    options
        .iter()
        .position(|option| *option == value)
        .unwrap_or(0)
}

fn join_tools(tools: &BTreeSet<String>) -> String {
    tools.iter().cloned().collect::<Vec<_>>().join(", ")
}

fn split_tools(text: &str) -> BTreeSet<String> {
    text.split(',')
        .map(str::trim)
        .filter(|tool| !tool.is_empty())
        .map(str::to_string)
        .collect()
}

/// The arguments as one line: separated by spaces, an argument with a
/// space or a quote in double quotes.
pub(super) fn join_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            if !argument.is_empty() && !argument.contains([' ', '\t', '"', '\'', '\\']) {
                argument.clone()
            } else {
                format!(
                    "\"{}\"",
                    argument.replace('\\', "\\\\").replace('"', "\\\"")
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The line read back: words split at spaces, a quoted part (single or
/// double quotes) kept whole, `\` escaping the next character in double
/// quotes.
pub(super) fn split_arguments(line: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote = None;
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match (quote, character) {
            (Some('"'), '\\') => current.extend(characters.next()),
            (Some(open), character) if character == open => quote = None,
            (Some(_), character) => current.push(character),
            (None, '"' | '\'') => {
                quote = Some(character);
                started = true;
            }
            (None, character) if character.is_whitespace() => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, character) => {
                current.push(character);
                started = true;
            }
        }
    }
    if started {
        arguments.push(current);
    }
    arguments
}
