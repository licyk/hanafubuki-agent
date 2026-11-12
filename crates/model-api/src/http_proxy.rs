//! The proxy the model API's HTTP clients go through: a Provider's
//! requests, its model listing and the models.dev download.

use reqwest::ClientBuilder;

use crate::port::{ModelError, ModelResult};

/// The proxy an HTTP client goes through.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum ModelHttpProxy {
    /// The operating system's proxy settings (and the proxy environment
    /// variables).
    #[default]
    System,
    /// No proxy.
    Disabled,
    /// This proxy URL.
    Explicit(String),
}

/// The proxy URL is not shown: it may hold credentials.
impl std::fmt::Debug for ModelHttpProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::System => "System",
            Self::Disabled => "Disabled",
            Self::Explicit(_) => "Explicit(<redacted>)",
        })
    }
}

impl ModelHttpProxy {
    /// `builder` going through this proxy.
    pub(crate) fn apply(&self, builder: ClientBuilder) -> ModelResult<ClientBuilder> {
        match self {
            Self::System => Ok(builder),
            Self::Disabled => Ok(builder.no_proxy()),
            // The URL may hold the proxy's credentials, so it is not named.
            Self::Explicit(address) => reqwest::Proxy::all(address)
                .map(|proxy| builder.proxy(proxy))
                .map_err(|_| ModelError::InvalidRequest("the proxy is not usable".into())),
        }
    }
}
