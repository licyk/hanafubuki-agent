use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use hanafubuki_agent_foundation::secrets::{MemorySecretStore, SecretKey, SecretStore};
use hanafubuki_agent_tool_contracts::canonical_digest;
use serde::{Deserialize, Serialize};

use crate::{
    McpConnectionSecrets, McpCredentialKey, McpError, McpResolvedSecretEnvironment,
    McpResolvedSecretHeaders, McpServerConfig, McpTransportConfig,
};

pub const MAX_MCP_SECRET_BYTES: usize = 64 * 1024;

pub type McpSecretFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, McpError>> + Send + 'a>>;

/// Backend port for a platform secure store. Implementations must never expose
/// values through snapshots, logs, errors, or IPC responses.
pub trait McpSecretStore: Send + Sync {
    fn get<'a>(&'a self, key: &'a McpCredentialKey) -> McpSecretFuture<'a, Option<Vec<u8>>>;
    fn set<'a>(&'a self, key: &'a McpCredentialKey, value: Vec<u8>) -> McpSecretFuture<'a, ()>;
    fn delete<'a>(&'a self, key: &'a McpCredentialKey) -> McpSecretFuture<'a, ()>;
}

#[derive(Clone, Default)]
pub struct MemoryMcpSecretStore {
    store: MemorySecretStore,
}

impl McpSecretStore for MemoryMcpSecretStore {
    fn get<'a>(&'a self, key: &'a McpCredentialKey) -> McpSecretFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            self.store
                .get(&mcp_secret_key(key)?)
                .await
                .map_err(secret_store_error)
        })
    }

    fn set<'a>(&'a self, key: &'a McpCredentialKey, value: Vec<u8>) -> McpSecretFuture<'a, ()> {
        Box::pin(async move {
            self.store
                .set(&mcp_secret_key(key)?, value)
                .await
                .map_err(secret_store_error)
        })
    }

    fn delete<'a>(&'a self, key: &'a McpCredentialKey) -> McpSecretFuture<'a, ()> {
        Box::pin(async move {
            self.store
                .delete(&mcp_secret_key(key)?)
                .await
                .map_err(secret_store_error)
        })
    }
}

/// MCP's typed-key adapter over the neutral platform secret mechanism.
#[derive(Clone)]
pub struct FoundationMcpSecretStore {
    store: Arc<dyn SecretStore>,
}

impl FoundationMcpSecretStore {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }
}

impl McpSecretStore for FoundationMcpSecretStore {
    fn get<'a>(&'a self, key: &'a McpCredentialKey) -> McpSecretFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            self.store
                .get(&mcp_secret_key(key)?)
                .await
                .map_err(secret_store_error)
        })
    }

    fn set<'a>(&'a self, key: &'a McpCredentialKey, value: Vec<u8>) -> McpSecretFuture<'a, ()> {
        Box::pin(async move {
            self.store
                .set(&mcp_secret_key(key)?, value)
                .await
                .map_err(secret_store_error)
        })
    }

    fn delete<'a>(&'a self, key: &'a McpCredentialKey) -> McpSecretFuture<'a, ()> {
        Box::pin(async move {
            self.store
                .delete(&mcp_secret_key(key)?)
                .await
                .map_err(secret_store_error)
        })
    }
}

fn mcp_secret_key(key: &McpCredentialKey) -> Result<SecretKey, McpError> {
    SecretKey::new(format!("mcp:{}", key.as_str())).map_err(secret_store_error)
}

fn secret_store_error(_error: impl std::fmt::Display) -> McpError {
    McpError::Unavailable("platform secure credential store operation failed".into())
}

/// A configured credential.
///
/// The value is the exact string the user entered; it is stored, returned, and
/// displayed verbatim. `generation` exists only so a changed value invalidates
/// live connections — it is never a substitute for the value itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCredentialSettings {
    pub value: String,
    pub generation: u64,
}

/// Resolve the credential values one server needs for a connection.
///
/// Reads straight from the durable settings map: there is no secure-store
/// round trip, so this is an ordinary synchronous lookup.
pub fn resolve_connection_secrets(
    config: &McpServerConfig,
    credentials: &BTreeMap<McpCredentialKey, McpCredentialSettings>,
) -> Result<McpConnectionSecrets, McpError> {
    let mut environment = BTreeMap::new();
    let mut headers = BTreeMap::new();
    let mut referenced = BTreeSet::new();
    match &config.transport {
        McpTransportConfig::Stdio(stdio) => {
            for (name, key) in &stdio.credential_environment {
                environment.insert(name.clone(), read_credential(key, credentials)?);
                referenced.insert(key.clone());
            }
        }
        McpTransportConfig::StreamableHttp(http) => {
            for (name, key) in &http.credential_headers {
                headers.insert(name.clone(), read_credential(key, credentials)?);
                referenced.insert(key.clone());
            }
        }
    }
    let reference_generations = referenced
        .iter()
        .map(|key| {
            (
                key.as_str(),
                credentials
                    .get(key)
                    .map(|credential| credential.generation)
                    .unwrap_or(0),
            )
        })
        .collect::<Vec<_>>();
    let digest = canonical_digest(&reference_generations).map_err(|_| McpError::LimitExceeded)?;
    let credential_reference_generation = u64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("digest prefix is eight bytes"),
    );
    Ok(McpConnectionSecrets {
        credential_reference_generation,
        stdio_environment: McpResolvedSecretEnvironment::new(environment)?,
        http_headers: McpResolvedSecretHeaders::new(headers)?,
    })
}

fn read_credential(
    key: &McpCredentialKey,
    credentials: &BTreeMap<McpCredentialKey, McpCredentialSettings>,
) -> Result<String, McpError> {
    credentials
        .get(key)
        .map(|credential| credential.value.clone())
        .ok_or_else(|| {
            McpError::Unavailable(format!(
                "required credential reference {} is missing",
                key.as_str()
            ))
        })
}

/// Bound the representation of a stored credential. The value is never
/// transformed: it is rejected or kept exactly as written.
pub fn validate_credential_value(value: &str) -> Result<(), McpError> {
    if value.is_empty() || value.len() > MAX_MCP_SECRET_BYTES || value.contains('\0') {
        return Err(McpError::InvalidConfig(
            "credential value must contain 1 to 65536 bytes without NUL".into(),
        ));
    }
    Ok(())
}
