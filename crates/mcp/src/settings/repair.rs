//! Per-entry repair of the persisted MCP settings document.
//!
//! The file is our own data, so it is read tolerantly: unknown fields are
//! dropped, a number outside its range is moved to the nearest bound, and an
//! entry that still cannot be used is dropped alone while every other entry is
//! kept. Each repair is reported without any value from the file; the caller
//! keeps the original before it writes the repaired document. The
//! security checks of `McpServerConfig::validate` (a URL without credentials,
//! no header or environment overlap, the stdio program, argument and
//! environment rules) are not loosened: a server that fails them is dropped.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    validate_credential_value, McpCredentialKey, McpCredentialSettings, McpServerConfig,
    McpServerId, McpSettings, MAX_MCP_CONCURRENT_CALLS, MAX_MCP_SERVERS, MAX_MCP_TIMEOUT_SECONDS,
};
use hanafubuki_agent_foundation::json_normalization::{
    normalize_current_json, JsonField, JsonShape, JsonVariant, OPAQUE_JSON_SHAPE,
};

const STDIO_TRANSPORT_FIELDS: &[JsonField] = &[
    JsonField::new("program", &OPAQUE_JSON_SHAPE),
    JsonField::new("argv", &OPAQUE_JSON_SHAPE),
    JsonField::new("cwd", &OPAQUE_JSON_SHAPE),
    JsonField::new("publicEnvironment", &OPAQUE_JSON_SHAPE),
    JsonField::new("credentialEnvironment", &OPAQUE_JSON_SHAPE),
];
const HTTP_TRANSPORT_FIELDS: &[JsonField] = &[
    JsonField::new("url", &OPAQUE_JSON_SHAPE),
    JsonField::new("publicHeaders", &OPAQUE_JSON_SHAPE),
    JsonField::new("credentialHeaders", &OPAQUE_JSON_SHAPE),
    JsonField::new("oauth", &OPAQUE_JSON_SHAPE),
];
const TRANSPORT_VARIANTS: &[JsonVariant] = &[
    JsonVariant::new("stdio", STDIO_TRANSPORT_FIELDS),
    JsonVariant::new("streamable_http", HTTP_TRANSPORT_FIELDS),
];
const TRANSPORT_SHAPE: JsonShape = JsonShape::TaggedObject {
    discriminator: "type",
    common: &[],
    variants: TRANSPORT_VARIANTS,
};
const SERVER_FIELDS: &[JsonField] = &[
    JsonField::new("id", &OPAQUE_JSON_SHAPE),
    JsonField::new("enabled", &OPAQUE_JSON_SHAPE),
    JsonField::new("required", &OPAQUE_JSON_SHAPE),
    JsonField::new("protocolMode", &OPAQUE_JSON_SHAPE),
    JsonField::new("transport", &TRANSPORT_SHAPE),
    JsonField::new("connectTimeoutSeconds", &OPAQUE_JSON_SHAPE),
    JsonField::new("initializeTimeoutSeconds", &OPAQUE_JSON_SHAPE),
    JsonField::new("listTimeoutSeconds", &OPAQUE_JSON_SHAPE),
    JsonField::new("callTimeoutSeconds", &OPAQUE_JSON_SHAPE),
    JsonField::new("shutdownTimeoutSeconds", &OPAQUE_JSON_SHAPE),
    JsonField::new("maximumConcurrentCalls", &OPAQUE_JSON_SHAPE),
    JsonField::new("toolMaximumConcurrentCalls", &OPAQUE_JSON_SHAPE),
    JsonField::new("enabledTools", &OPAQUE_JSON_SHAPE),
    JsonField::new("disabledTools", &OPAQUE_JSON_SHAPE),
    JsonField::new("approvalPolicy", &OPAQUE_JSON_SHAPE),
    JsonField::new("toolApprovalPolicies", &OPAQUE_JSON_SHAPE),
    JsonField::new("exposurePolicy", &OPAQUE_JSON_SHAPE),
    JsonField::new("toolExposurePolicies", &OPAQUE_JSON_SHAPE),
];
const SERVER_SHAPE: JsonShape = JsonShape::Object(SERVER_FIELDS);
const SERVERS_SHAPE: JsonShape = JsonShape::Array(&SERVER_SHAPE);
const CREDENTIAL_FIELDS: &[JsonField] = &[
    JsonField::new("value", &OPAQUE_JSON_SHAPE),
    JsonField::new("generation", &OPAQUE_JSON_SHAPE),
];
const CREDENTIAL_SHAPE: JsonShape = JsonShape::Object(CREDENTIAL_FIELDS);
const CREDENTIALS_SHAPE: JsonShape = JsonShape::MapValues(&CREDENTIAL_SHAPE);
const SETTINGS_FIELDS: &[JsonField] = &[
    JsonField::new("revision", &OPAQUE_JSON_SHAPE),
    JsonField::new("servers", &SERVERS_SHAPE),
    JsonField::new("credentials", &CREDENTIALS_SHAPE),
];
const SETTINGS_SHAPE: JsonShape = JsonShape::Object(SETTINGS_FIELDS);

const TIMEOUT_FIELDS: [&str; 5] = [
    "connectTimeoutSeconds",
    "initializeTimeoutSeconds",
    "listTimeoutSeconds",
    "callTimeoutSeconds",
    "shutdownTimeoutSeconds",
];

/// One repair made while reading the MCP settings file.
///
/// It names what was repaired, never a value from the file, so it can be shown
/// beside the settings and written to the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSettingsRepair {
    pub kind: McpSettingsRepairKind,
    /// The server id or credential key the repair concerns, when the entry
    /// carries a valid one.
    pub subject: Option<String>,
    /// The field a clamp changed.
    pub field: Option<String>,
    /// Why the entry was repaired.
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpSettingsRepairKind {
    /// The document could not be read and was replaced by the defaults.
    Reset,
    /// A number outside its range was moved to the nearest bound.
    ValueClamped,
    /// A server entry could not be kept.
    ServerDropped,
    /// A credential entry could not be kept.
    CredentialDropped,
}

impl McpSettingsRepair {
    /// The document could not be read at all (`detail` says why) and the
    /// defaults replaced it.
    pub fn reset(detail: &str) -> Self {
        Self {
            kind: McpSettingsRepairKind::Reset,
            subject: None,
            field: None,
            detail: detail.to_string(),
        }
    }

    fn dropped(kind: McpSettingsRepairKind, subject: Option<String>, detail: String) -> Self {
        Self {
            kind,
            subject,
            field: None,
            detail,
        }
    }

    fn clamped(subject: Option<String>, field: &str, minimum: u64, maximum: u64) -> Self {
        Self {
            kind: McpSettingsRepairKind::ValueClamped,
            subject,
            field: Some(field.to_string()),
            detail: format!("must be a whole number from {minimum} to {maximum}"),
        }
    }
}

/// Repair a parsed settings document entry by entry.
pub fn repair_settings(mut raw: Value) -> (McpSettings, Vec<McpSettingsRepair>) {
    normalize_current_json(&mut raw, &SETTINGS_SHAPE, |_| {});
    let Some(mut object) = raw.as_object().cloned() else {
        return (
            McpSettings::default(),
            vec![McpSettingsRepair::reset(
                "the settings file is not a JSON object",
            )],
        );
    };
    let mut repairs = Vec::new();
    let revision = object
        .get("revision")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1);
    let servers = repair_servers(
        object
            .remove("servers")
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default(),
        &mut repairs,
    );
    let credentials = repair_credentials(
        object
            .remove("credentials")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        &mut repairs,
    );
    (
        McpSettings {
            revision,
            servers,
            credentials,
        },
        repairs,
    )
}

fn repair_servers(
    values: Vec<Value>,
    repairs: &mut Vec<McpSettingsRepair>,
) -> Vec<McpServerConfig> {
    let mut servers = Vec::new();
    let mut server_ids = HashSet::new();
    for mut value in values {
        let subject = server_subject(&value);
        let dropped = |detail: String| {
            McpSettingsRepair::dropped(
                McpSettingsRepairKind::ServerDropped,
                subject.clone(),
                detail,
            )
        };
        if servers.len() == MAX_MCP_SERVERS {
            repairs.push(dropped(format!(
                "past the limit of {MAX_MCP_SERVERS} servers"
            )));
            continue;
        }
        clamp_server_numbers(&mut value, &subject, repairs);
        let Ok(server) = serde_json::from_value::<McpServerConfig>(value) else {
            repairs.push(dropped(
                "the entry is not a readable MCP server".to_string(),
            ));
            continue;
        };
        if let Err(error) = server.validate() {
            repairs.push(dropped(error.to_string()));
            continue;
        }
        if !server_ids.insert(server.id.clone()) {
            repairs.push(dropped("another server has the same id".to_string()));
            continue;
        }
        servers.push(server);
    }
    servers
}

fn repair_credentials(
    values: Map<String, Value>,
    repairs: &mut Vec<McpSettingsRepair>,
) -> BTreeMap<McpCredentialKey, McpCredentialSettings> {
    let mut credentials = BTreeMap::new();
    for (key, mut value) in values {
        let Ok(key) = McpCredentialKey::parse(key) else {
            repairs.push(McpSettingsRepair::dropped(
                McpSettingsRepairKind::CredentialDropped,
                None,
                "the credential key is invalid".to_string(),
            ));
            continue;
        };
        let subject = Some(key.as_str().to_string());
        let dropped = |detail: String| {
            McpSettingsRepair::dropped(
                McpSettingsRepairKind::CredentialDropped,
                subject.clone(),
                detail,
            )
        };
        // A generation is only change detection; the value it guards is kept.
        if let Some(generation) = value.get_mut("generation") {
            if clamp_number(generation, 1, u64::MAX) {
                repairs.push(McpSettingsRepair::clamped(
                    subject.clone(),
                    "generation",
                    1,
                    u64::MAX,
                ));
            }
        }
        let Ok(credential) = serde_json::from_value::<McpCredentialSettings>(value) else {
            repairs.push(dropped(
                "the entry is not a readable credential".to_string(),
            ));
            continue;
        };
        if let Err(error) = validate_credential_value(&credential.value) {
            repairs.push(dropped(error.to_string()));
            continue;
        }
        credentials.insert(key, credential);
    }
    credentials
}

fn server_subject(value: &Value) -> Option<String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| McpServerId::parse(id).ok())
        .map(|id| id.as_str().to_string())
}

fn clamp_server_numbers(
    value: &mut Value,
    subject: &Option<String>,
    repairs: &mut Vec<McpSettingsRepair>,
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let calls = MAX_MCP_CONCURRENT_CALLS as u64;
    let mut clamp = |object: &mut Map<String, Value>, field: &str, maximum: u64| {
        if object
            .get_mut(field)
            .is_some_and(|value| clamp_number(value, 1, maximum))
        {
            repairs.push(McpSettingsRepair::clamped(
                subject.clone(),
                field,
                1,
                maximum,
            ));
        }
    };
    for field in TIMEOUT_FIELDS {
        clamp(object, field, MAX_MCP_TIMEOUT_SECONDS);
    }
    clamp(object, "maximumConcurrentCalls", calls);
    let field = "toolMaximumConcurrentCalls";
    if let Some(Value::Object(per_tool)) = object.get_mut(field) {
        let mut changed = false;
        for value in per_tool.values_mut() {
            changed |= clamp_number(value, 1, calls);
        }
        if changed {
            repairs.push(McpSettingsRepair::clamped(subject.clone(), field, 1, calls));
        }
    }
}

/// Move a JSON number into `minimum..=maximum` as a whole number.
///
/// Returns whether the value changed. Anything that is not a number is left
/// for the decoder to refuse.
fn clamp_number(value: &mut Value, minimum: u64, maximum: u64) -> bool {
    let Some(number) = value.as_number() else {
        return false;
    };
    let clamped = if let Some(whole) = number.as_u64() {
        whole.clamp(minimum, maximum)
    } else if number.is_i64() {
        minimum
    } else {
        match number.as_f64() {
            Some(real) if real > minimum as f64 && real < maximum as f64 => real as u64,
            Some(real) if real >= maximum as f64 => maximum,
            _ => minimum,
        }
    };
    if number.as_u64() == Some(clamped) {
        return false;
    }
    *value = Value::from(clamped);
    true
}
