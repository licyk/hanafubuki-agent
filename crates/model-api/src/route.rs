//! How the Agent names a model as it reaches it: by protocol, endpoint and
//! model id.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ModelApiProtocol, ModelEndpoint};

/// A Provider's endpoint as one protocol reaches it: what a model listing
/// describes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEndpointKey {
    protocol: ModelApiProtocol,
    /// The canonical base URL's SHA-256, in hex, not the URL: the key is
    /// persisted and logged, and a base URL's query may hold a credential.
    base_url_sha256: String,
}

impl ModelEndpointKey {
    pub fn new(protocol: ModelApiProtocol, endpoint: &ModelEndpoint) -> Self {
        let digest: [u8; 32] = Sha256::digest(endpoint.canonical_identity().as_bytes()).into();
        Self {
            protocol,
            base_url_sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        }
    }

    /// `model` at this endpoint.
    pub fn route(&self, model: &str) -> ModelRouteKey {
        ModelRouteKey {
            endpoint: self.clone(),
            model: model.trim().to_string(),
        }
    }
}

/// A model as the Agent reaches it: its endpoint and its id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRouteKey {
    endpoint: ModelEndpointKey,
    model: String,
}

impl ModelRouteKey {
    pub fn new(protocol: ModelApiProtocol, endpoint: &ModelEndpoint, model: &str) -> Self {
        ModelEndpointKey::new(protocol, endpoint).route(model)
    }

    pub fn endpoint(&self) -> &ModelEndpointKey {
        &self.endpoint
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests;
