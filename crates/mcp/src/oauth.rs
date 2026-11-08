use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_tool_contracts::canonical_digest;
use parking_lot::Mutex;
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationRequest, CredentialStore, OAuthHttpClient,
    OAuthState, StoredCredentials,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    McpCredentialKey, McpError, McpOAuthPolicy, McpSecretStore, McpServerConfig, McpServerId,
    McpTransportConfig, MAX_MCP_SECRET_BYTES,
};

const MAX_OAUTH_CALLBACK_BYTES: usize = 16 * 1024;
const MAX_OAUTH_REDIRECT_BYTES: usize = 4 * 1024;
const MAX_OAUTH_FLOW_AGE: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct McpOAuthFlowId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthPhase {
    Unsupported,
    LoggedOut,
    Authorizing,
    Authorized,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStatus {
    pub server_id: McpServerId,
    pub phase: McpOAuthPhase,
    pub flow_id: Option<McpOAuthFlowId>,
    pub credential_generation: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthLogin {
    pub server_id: McpServerId,
    pub flow_id: McpOAuthFlowId,
    pub authorization_url: String,
}

struct OAuthEntry {
    flow_id: McpOAuthFlowId,
    redirect_uri: url::Url,
    state: OAuthState,
    created_at: Instant,
}

#[derive(Clone)]
pub struct McpOAuthManager {
    /// OAuth tokens are issued by the remote server at runtime, not typed by
    /// the user, so they stay in the platform secure store rather than the
    /// user-editable settings file.
    credentials: Arc<dyn McpSecretStore>,
    /// Replaceable so a proxy change reaches later OAuth requests.
    http_client: Arc<parking_lot::RwLock<Arc<dyn OAuthHttpClient>>>,
    entries: Arc<Mutex<HashMap<McpServerId, Arc<AsyncMutex<OAuthEntry>>>>>,
    statuses: Arc<Mutex<HashMap<McpServerId, McpOAuthStatus>>>,
    next_flow_id: Arc<AtomicU64>,
}

impl McpOAuthManager {
    pub fn new(credentials: Arc<dyn McpSecretStore>, http_client: reqwest::Client) -> Self {
        Self {
            credentials,
            http_client: Arc::new(parking_lot::RwLock::new(Arc::new(
                crate::client_adapter::McpOAuthHttpClient::new(http_client),
            ))),
            entries: Arc::new(Mutex::new(HashMap::new())),
            statuses: Arc::new(Mutex::new(HashMap::new())),
            next_flow_id: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The client later OAuth requests use, after a proxy change.
    pub fn set_http_client(&self, http_client: reqwest::Client) {
        *self.http_client.write() =
            Arc::new(crate::client_adapter::McpOAuthHttpClient::new(http_client));
    }

    fn oauth_http_client(&self) -> Arc<dyn OAuthHttpClient> {
        self.http_client.read().clone()
    }

    pub fn status(&self, config: &McpServerConfig) -> McpOAuthStatus {
        if oauth_http_config(config).is_err() {
            return McpOAuthStatus {
                server_id: config.id.clone(),
                phase: McpOAuthPhase::Unsupported,
                flow_id: None,
                credential_generation: 0,
                last_error: None,
            };
        }
        self.statuses
            .lock()
            .get(&config.id)
            .cloned()
            .unwrap_or(McpOAuthStatus {
                server_id: config.id.clone(),
                phase: McpOAuthPhase::LoggedOut,
                flow_id: None,
                credential_generation: 0,
                last_error: None,
            })
    }

    pub async fn start_login(
        &self,
        config: &McpServerConfig,
        redirect_uri: String,
    ) -> Result<McpOAuthLogin, McpError> {
        let http = oauth_http_config(config)?;
        let redirect = validate_redirect_uri(&redirect_uri)?;
        let store =
            OAuthCredentialStore::new(self.credentials.clone(), oauth_credential_key(&config.id)?);
        let mut manager =
            AuthorizationManager::new_with_oauth_http_client(&http.url, self.oauth_http_client())
                .await
                .map_err(oauth_error)?;
        manager.set_credential_store(store);
        let mut state = OAuthState::Unauthorized(manager);
        state
            .start_authorization(
                AuthorizationRequest::new(redirect.as_str()).with_client_name("Hanafubuki"),
            )
            .await
            .map_err(oauth_error)?;
        let authorization_url = state.get_authorization_url().await.map_err(oauth_error)?;
        let flow_id = McpOAuthFlowId(
            self.next_flow_id
                .fetch_add(1, Ordering::Relaxed)
                .checked_add(1)
                .ok_or(McpError::LimitExceeded)?,
        );
        self.entries.lock().insert(
            config.id.clone(),
            Arc::new(AsyncMutex::new(OAuthEntry {
                flow_id,
                redirect_uri: redirect,
                state,
                created_at: Instant::now(),
            })),
        );
        let generation = self.status(config).credential_generation;
        self.statuses.lock().insert(
            config.id.clone(),
            McpOAuthStatus {
                server_id: config.id.clone(),
                phase: McpOAuthPhase::Authorizing,
                flow_id: Some(flow_id),
                credential_generation: generation,
                last_error: None,
            },
        );
        Ok(McpOAuthLogin {
            server_id: config.id.clone(),
            flow_id,
            authorization_url,
        })
    }

    pub async fn complete_login(
        &self,
        config: &McpServerConfig,
        flow_id: McpOAuthFlowId,
        callback_url: String,
    ) -> Result<McpOAuthStatus, McpError> {
        oauth_http_config(config)?;
        let entry = self
            .entries
            .lock()
            .get(&config.id)
            .cloned()
            .ok_or(McpError::StaleGeneration)?;
        let mut entry = entry.lock().await;
        if entry.flow_id != flow_id {
            return Err(McpError::StaleGeneration);
        }
        if entry.created_at.elapsed() > MAX_OAUTH_FLOW_AGE {
            drop(entry);
            self.entries.lock().remove(&config.id);
            self.set_failed(config, flow_id);
            return Err(McpError::Unavailable("MCP OAuth flow expired".into()));
        }
        validate_callback(&entry.redirect_uri, &callback_url)?;
        if let Err(error) = entry.state.handle_callback_url(&callback_url).await {
            drop(entry);
            self.entries.lock().remove(&config.id);
            self.set_failed(config, flow_id);
            return Err(oauth_error(error));
        }
        self.entries.lock().remove(&config.id);
        let generation = self
            .status(config)
            .credential_generation
            .checked_add(1)
            .ok_or(McpError::LimitExceeded)?;
        let status = McpOAuthStatus {
            server_id: config.id.clone(),
            phase: McpOAuthPhase::Authorized,
            flow_id: None,
            credential_generation: generation,
            last_error: None,
        };
        self.statuses
            .lock()
            .insert(config.id.clone(), status.clone());
        Ok(status)
    }

    pub fn cancel_login(
        &self,
        config: &McpServerConfig,
        flow_id: McpOAuthFlowId,
    ) -> Result<McpOAuthStatus, McpError> {
        let entry = self
            .entries
            .lock()
            .remove(&config.id)
            .ok_or(McpError::StaleGeneration)?;
        if entry
            .try_lock()
            .map_or(true, |entry| entry.flow_id != flow_id)
        {
            self.entries.lock().insert(config.id.clone(), entry);
            return Err(McpError::StaleGeneration);
        }
        let generation = self.status(config).credential_generation;
        let status = McpOAuthStatus {
            server_id: config.id.clone(),
            phase: McpOAuthPhase::LoggedOut,
            flow_id: None,
            credential_generation: generation,
            last_error: None,
        };
        self.statuses
            .lock()
            .insert(config.id.clone(), status.clone());
        Ok(status)
    }

    pub async fn logout(&self, config: &McpServerConfig) -> Result<McpOAuthStatus, McpError> {
        oauth_http_config(config)?;
        self.entries.lock().remove(&config.id);
        self.credentials
            .delete(&oauth_credential_key(&config.id)?)
            .await?;
        let generation = self
            .status(config)
            .credential_generation
            .checked_add(1)
            .ok_or(McpError::LimitExceeded)?;
        let status = McpOAuthStatus {
            server_id: config.id.clone(),
            phase: McpOAuthPhase::LoggedOut,
            flow_id: None,
            credential_generation: generation,
            last_error: None,
        };
        self.statuses
            .lock()
            .insert(config.id.clone(), status.clone());
        Ok(status)
    }

    pub async fn refresh(&self, config: &McpServerConfig) -> Result<McpOAuthStatus, McpError> {
        let key = oauth_credential_key(&config.id)?;
        let previous = self.credentials.get(&key).await?;
        let manager = self.authorized_manager(config).await?;
        if let Err(error) = manager.refresh_token().await {
            match previous {
                Some(previous) => self.credentials.set(&key, previous).await?,
                None => self.credentials.delete(&key).await?,
            }
            return Err(oauth_error(error));
        }
        let generation = self
            .status(config)
            .credential_generation
            .checked_add(1)
            .ok_or(McpError::LimitExceeded)?;
        let status = McpOAuthStatus {
            server_id: config.id.clone(),
            phase: McpOAuthPhase::Authorized,
            flow_id: None,
            credential_generation: generation,
            last_error: None,
        };
        self.statuses
            .lock()
            .insert(config.id.clone(), status.clone());
        Ok(status)
    }

    pub async fn access_token(&self, config: &McpServerConfig) -> Result<Option<String>, McpError> {
        let McpTransportConfig::StreamableHttp(http) = &config.transport else {
            return Err(McpError::Unsupported(
                "OAuth is available only for Streamable HTTP MCP servers".into(),
            ));
        };
        if http.oauth == McpOAuthPolicy::Disabled {
            return Ok(None);
        }
        match self.authorized_manager(config).await {
            Ok(manager) => {
                let token = manager.get_access_token().await.map_err(oauth_error)?;
                let generation = self.status(config).credential_generation.max(1);
                self.statuses.lock().insert(
                    config.id.clone(),
                    McpOAuthStatus {
                        server_id: config.id.clone(),
                        phase: McpOAuthPhase::Authorized,
                        flow_id: None,
                        credential_generation: generation,
                        last_error: None,
                    },
                );
                Ok(Some(token))
            }
            Err(_error) if http.oauth == McpOAuthPolicy::Optional => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn set_failed(&self, config: &McpServerConfig, flow_id: McpOAuthFlowId) {
        let generation = self.status(config).credential_generation;
        self.statuses.lock().insert(
            config.id.clone(),
            McpOAuthStatus {
                server_id: config.id.clone(),
                phase: McpOAuthPhase::Failed,
                flow_id: Some(flow_id),
                credential_generation: generation,
                last_error: Some("OAuth authorization failed".into()),
            },
        );
    }

    async fn authorized_manager(
        &self,
        config: &McpServerConfig,
    ) -> Result<AuthorizationManager, McpError> {
        let http = oauth_http_config(config)?;
        let mut manager =
            AuthorizationManager::new_with_oauth_http_client(&http.url, self.oauth_http_client())
                .await
                .map_err(oauth_error)?;
        manager.set_credential_store(OAuthCredentialStore::new(
            self.credentials.clone(),
            oauth_credential_key(&config.id)?,
        ));
        if !manager.initialize_from_store().await.map_err(oauth_error)? {
            return Err(McpError::Unavailable("MCP OAuth login is required".into()));
        }
        Ok(manager)
    }
}

#[derive(Clone)]
struct OAuthCredentialStore {
    store: Arc<dyn McpSecretStore>,
    key: McpCredentialKey,
}

impl OAuthCredentialStore {
    fn new(store: Arc<dyn McpSecretStore>, key: McpCredentialKey) -> Self {
        Self { store, key }
    }
}

#[async_trait::async_trait]
impl CredentialStore for OAuthCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let Some(encoded) = self.store.get(&self.key).await.map_err(store_error)? else {
            return Ok(None);
        };
        serde_json::from_slice(&encoded)
            .map(Some)
            .map_err(|_| AuthError::InternalError("stored OAuth credential is invalid".into()))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let encoded = serde_json::to_vec(&credentials)
            .map_err(|_| AuthError::InternalError("OAuth credential cannot be encoded".into()))?;
        if encoded.len() > MAX_MCP_SECRET_BYTES {
            return Err(AuthError::InternalError(
                "OAuth credential exceeds the storage limit".into(),
            ));
        }
        self.store
            .set(&self.key, encoded)
            .await
            .map_err(store_error)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.store.delete(&self.key).await.map_err(store_error)
    }
}

fn oauth_http_config(
    config: &McpServerConfig,
) -> Result<&crate::McpStreamableHttpConfig, McpError> {
    let McpTransportConfig::StreamableHttp(http) = &config.transport else {
        return Err(McpError::Unsupported(
            "OAuth is available only for Streamable HTTP MCP servers".into(),
        ));
    };
    if http.oauth == McpOAuthPolicy::Disabled {
        return Err(McpError::Unsupported(
            "OAuth is disabled for this MCP server".into(),
        ));
    }
    Ok(http)
}

fn validate_redirect_uri(value: &str) -> Result<url::Url, McpError> {
    if value.len() > MAX_OAUTH_REDIRECT_BYTES {
        return Err(McpError::LimitExceeded);
    }
    let redirect = url::Url::parse(value)
        .map_err(|_| McpError::InvalidConfig("OAuth redirect URI is invalid".into()))?;
    if !matches!(redirect.scheme(), "http" | "https")
        || redirect.host_str().is_none()
        || !redirect.username().is_empty()
        || redirect.password().is_some()
        || redirect.query().is_some()
        || redirect.fragment().is_some()
    {
        return Err(McpError::InvalidConfig(
            "OAuth redirect URI must be an HTTP(S) URL without credentials, query, or fragment"
                .into(),
        ));
    }
    Ok(redirect)
}

fn validate_callback(redirect: &url::Url, callback: &str) -> Result<(), McpError> {
    if callback.len() > MAX_OAUTH_CALLBACK_BYTES {
        return Err(McpError::LimitExceeded);
    }
    let callback = url::Url::parse(callback)
        .map_err(|_| McpError::InvalidConfig("OAuth callback URL is invalid".into()))?;
    if callback.scheme() != redirect.scheme()
        || callback.host_str() != redirect.host_str()
        || callback.port_or_known_default() != redirect.port_or_known_default()
        || callback.path() != redirect.path()
        || !callback.username().is_empty()
        || callback.password().is_some()
        || callback.fragment().is_some()
    {
        return Err(McpError::InvalidConfig(
            "OAuth callback does not match the active redirect URI".into(),
        ));
    }
    Ok(())
}

fn oauth_credential_key(server_id: &McpServerId) -> Result<McpCredentialKey, McpError> {
    let digest = canonical_digest(server_id).map_err(|_| McpError::LimitExceeded)?;
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    McpCredentialKey::parse(format!("oauth.{suffix}"))
}

fn oauth_error(_error: AuthError) -> McpError {
    McpError::Unavailable("MCP OAuth operation failed".into())
}

fn store_error(_error: McpError) -> AuthError {
    AuthError::InternalError("secure credential store operation failed".into())
}

#[cfg(test)]
mod tests;
