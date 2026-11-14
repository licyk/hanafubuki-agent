//! The release lookup: one `GET` of GitHub's latest release, with the
//! binary's User-Agent, a token only when the caller gives one, the proxy
//! of `network.proxy`, and its own time and size bounds.

#[cfg(debug_assertions)]
use std::ffi::OsStr;
use std::fmt;
use std::time::Duration;

use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION};
use reqwest::{ClientBuilder, StatusCode, Url};

use crate::error::UpdateError;
use crate::release::{is_loopback_http, Release};

/// GitHub's newest release that is neither a draft nor a pre-release. Only
/// a release build has it: a debug build never contacts GitHub.
#[cfg(not(debug_assertions))]
const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/licyk/hanafubuki-agent/releases/latest";
/// The binary and its version, and nothing of the person.
const USER_AGENT: &str = concat!("hanafubuki-agent/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The whole lookup, the answer's body included.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// reqwest's own bound.
const MAX_REDIRECTS: usize = 10;
/// A debug build's only release API (D16); a release build has no such
/// variable.
#[cfg(debug_assertions)]
const TEST_RELEASE_URL: &str = "HANAFUBUKI_AGENT_TEST_RELEASE_URL";
/// The addresses an explicit proxy never takes.
const LOOPBACK: &str = "localhost,127.0.0.0/8,::1";

/// The proxy the lookup goes through, from `network.proxy`'s string form.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum UpdateProxy {
    /// The operating system's proxy settings and the proxy variables.
    #[default]
    System,
    /// No proxy.
    Disabled,
    /// This proxy URL, for every host but this machine's loopback addresses.
    Explicit(String),
}

impl UpdateProxy {
    /// `"system"`, `"disabled"`, or else the proxy's URL.
    pub fn from_setting(value: &str) -> Self {
        match value.trim() {
            "system" => Self::System,
            "disabled" => Self::Disabled,
            url => Self::Explicit(url.to_string()),
        }
    }

    fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder, UpdateError> {
        match self {
            Self::System => Ok(builder),
            Self::Disabled => Ok(builder.no_proxy()),
            Self::Explicit(url) => reqwest::Proxy::all(url)
                .map(|proxy| builder.proxy(proxy.no_proxy(reqwest::NoProxy::from_string(LOOPBACK))))
                .map_err(|_| UpdateError::Proxy),
        }
    }
}

/// The proxy URL is not shown: it may hold credentials.
impl fmt::Debug for UpdateProxy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::System => "System",
            Self::Disabled => "Disabled",
            Self::Explicit(_) => "Explicit(<redacted>)",
        })
    }
}

/// Looks up the latest release.
#[derive(Clone)]
pub struct ReleaseClient {
    http: reqwest::Client,
    endpoint: Url,
    /// Whether an asset may be plain `http` to a loopback address: only
    /// through the test release URL.
    loopback_http: bool,
    /// `Bearer <token>`, marked sensitive.
    token: Option<HeaderValue>,
    lookup_timeout: Duration,
}

/// Neither the token nor a proxy is shown.
impl fmt::Debug for ReleaseClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReleaseClient")
            .field("endpoint", &self.endpoint.as_str())
            .field("token", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

impl ReleaseClient {
    /// GitHub's API in a release build; in a debug build the test release
    /// URL, or [`UpdateError::DebugBuild`] without it (D16). `token`
    /// (`GITHUB_TOKEN`) is sent to the API only, and only when given.
    pub fn for_this_build(proxy: &UpdateProxy, token: Option<String>) -> Result<Self, UpdateError> {
        let (endpoint, loopback_http) = this_build_endpoint()?;
        Self::new(endpoint, loopback_http, proxy, token, LOOKUP_TIMEOUT)
    }

    pub(crate) fn new(
        endpoint: Url,
        loopback_http: bool,
        proxy: &UpdateProxy,
        token: Option<String>,
        lookup_timeout: Duration,
    ) -> Result<Self, UpdateError> {
        let token = token
            .filter(|token| !token.is_empty())
            .map(|token| bearer(&token))
            .transpose()?;
        // Without a pool, a connection never outlives the runtime of the
        // lookup that opened it (`latest_blocking` makes one per call).
        let builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_max_idle_per_host(0)
            .redirect(redirect_policy(loopback_http))
            .user_agent(USER_AGENT);
        let http = proxy
            .apply(builder)?
            .build()
            .map_err(|_| UpdateError::Client)?;
        Ok(Self {
            http,
            endpoint,
            loopback_http,
            token,
            lookup_timeout,
        })
    }

    /// The latest release, within the lookup's time bound.
    pub async fn latest(&self) -> Result<Release, UpdateError> {
        tokio::time::timeout(self.lookup_timeout, self.fetch_latest())
            .await
            .unwrap_or_else(|_| Err(self.unreachable(true)))
    }

    /// [`Self::latest`] on a runtime of its own, for a thread that is not
    /// in one (it panics inside a Tokio runtime, as `block_on` does).
    pub fn latest_blocking(&self) -> Result<Release, UpdateError> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| UpdateError::Runtime {
                reason: error.to_string(),
            })?
            .block_on(self.latest())
    }

    async fn fetch_latest(&self) -> Result<Release, UpdateError> {
        let mut request = self
            .http
            .get(self.endpoint.clone())
            .header(ACCEPT, "application/vnd.github+json");
        if let Some(token) = &self.token {
            request = request.header(AUTHORIZATION, token.clone());
        }
        let failed = |error: reqwest::Error| self.unreachable(error.is_timeout());
        let mut response = request.send().await.map_err(failed)?;
        if let Some(error) = status_error(response.status()) {
            return Err(error);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(failed)? {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(UpdateError::NotUnderstood {
                    reason: "it is longer than 1 MiB".to_string(),
                });
            }
            body.extend_from_slice(&chunk);
        }
        Release::from_api_json(&body, self.loopback_http)
    }

    fn unreachable(&self, timed_out: bool) -> UpdateError {
        UpdateError::Unreachable {
            host: self.endpoint.host_str().unwrap_or_default().to_string(),
            timed_out,
        }
    }

    /// The HTTP client, for the archive's download: it carries no token
    /// (only the lookup's request adds it).
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

/// Every hop of a redirect is held to the asset URL's rule: `https`, or
/// `http` to a loopback address when the test release URL allows it.
fn redirect_policy(loopback_http: bool) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        let url = attempt.url();
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if url.scheme() == "https" || loopback_http && is_loopback_http(url) {
            attempt.follow()
        } else {
            attempt.error("a redirect to a URL that is not https")
        }
    })
}

fn bearer(token: &str) -> Result<HeaderValue, UpdateError> {
    let mut value =
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| UpdateError::InvalidToken)?;
    value.set_sensitive(true);
    Ok(value)
}

fn status_error(status: StatusCode) -> Option<UpdateError> {
    match status.as_u16() {
        200..=299 => None,
        401 => Some(UpdateError::TokenRejected),
        403 | 429 => Some(UpdateError::RateLimited {
            status: status.as_u16(),
        }),
        404 => Some(UpdateError::NoRelease),
        status => Some(UpdateError::Status { status }),
    }
}

/// A release build always asks GitHub: nothing in its environment can
/// redirect the lookup.
#[cfg(not(debug_assertions))]
fn this_build_endpoint() -> Result<(Url, bool), UpdateError> {
    let endpoint = Url::parse(LATEST_RELEASE_API).map_err(|_| UpdateError::Client)?;
    Ok((endpoint, false))
}

#[cfg(debug_assertions)]
fn this_build_endpoint() -> Result<(Url, bool), UpdateError> {
    test_endpoint(std::env::var_os(TEST_RELEASE_URL).as_deref())
}

/// The test release URL `value`, and whether its assets may then be plain
/// `http` to a loopback address: `https` anywhere, or `http` to a loopback
/// address.
#[cfg(debug_assertions)]
fn test_endpoint(value: Option<&OsStr>) -> Result<(Url, bool), UpdateError> {
    let unusable = |reason: &str| UpdateError::TestUrl {
        reason: reason.to_string(),
    };
    let text = match value {
        None => return Err(UpdateError::DebugBuild),
        Some(value) => value.to_str().ok_or_else(|| unusable("it is not UTF-8"))?,
    };
    let text = text.trim();
    if text.is_empty() {
        return Err(UpdateError::DebugBuild);
    }
    let endpoint = Url::parse(text).map_err(|_| unusable("it is not a URL"))?;
    if endpoint.scheme() != "https" && !is_loopback_http(&endpoint) {
        return Err(unusable(
            "it is neither https nor http to a loopback address",
        ));
    }
    Ok((endpoint, true))
}

#[cfg(test)]
mod tests;
