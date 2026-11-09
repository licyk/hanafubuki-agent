use serde::{Deserialize, Serialize};

use crate::McpError;

macro_rules! bounded_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, McpError> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > MAX_ID_BYTES
                    || matches!(value.as_str(), "." | "..")
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
                {
                    return Err(McpError::InvalidConfig("identifier is invalid".into()));
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

bounded_id!(McpServerId);
bounded_id!(McpCredentialKey);

const MAX_ID_BYTES: usize = 128;

impl McpCredentialKey {
    /// The key a settings form stores a server's secret environment variable
    /// or header `name` under: `<server>-<name>`, every character a key
    /// cannot hold replaced by `_` and the whole cut to a key's length.
    pub fn for_server_secret(server: &McpServerId, name: &str) -> Self {
        let mut key = format!("{}-", server.as_str());
        key.extend(name.chars().map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        }));
        // Every character is ASCII now, so any length is a char boundary;
        // the cut keeps at least the server id, which is a valid key.
        key.truncate(MAX_ID_BYTES);
        Self(key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct McpConnectionGeneration(pub [u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct McpCatalogGeneration(pub [u8; 32]);
