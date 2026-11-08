//! The one check for a URL the application stores, sends requests to, or
//! renders as a link.
//!
//! Every such URL has a scheme from a short list and a host; whether it may
//! also carry credentials, a query, or a fragment depends on what it is for,
//! so each caller names a [`UrlPolicy`] instead of restating the checks. The
//! message a rejection is reported with stays the caller's: the policy says
//! which component was refused, the caller says for which field.

use url::Url;

/// The schemes an HTTP(S) URL may use.
pub const HTTP_SCHEMES: &[&str] = &["http", "https"];

/// The longest URL the application keeps as an endpoint or takes from a
/// page: a search endpoint, a search result, a fetched page or a link in it.
pub const MAX_URL_BYTES: usize = 2048;

/// What a URL may carry besides its scheme and host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UrlPolicy {
    pub schemes: &'static [&'static str],
    pub credentials: bool,
    pub query: bool,
    pub fragment: bool,
    /// Checked on the text [`UrlPolicy::parse`] receives.
    pub max_bytes: Option<usize>,
}

/// Why a URL was refused, one component at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlRejection {
    TooLong { limit: usize },
    Invalid(url::ParseError),
    Scheme,
    MissingHost,
    Credentials,
    Query,
    Fragment,
}

impl UrlPolicy {
    /// HTTP(S) with a host, and anything else a URL may carry.
    pub const HTTP: Self = Self {
        schemes: HTTP_SCHEMES,
        credentials: true,
        query: true,
        fragment: true,
        max_bytes: None,
    };

    /// HTTP(S) without credentials: a page or a link the application opens.
    pub const PAGE: Self = Self {
        credentials: false,
        ..Self::HTTP
    };

    /// An address requests are built on: no credentials, query or fragment.
    pub const ENDPOINT: Self = Self {
        query: false,
        fragment: false,
        ..Self::PAGE
    };

    /// The same policy with a bound on the text's length.
    pub const fn within(self, max_bytes: usize) -> Self {
        Self {
            max_bytes: Some(max_bytes),
            ..self
        }
    }

    /// Parses `raw` and checks it against the policy. The text is taken as
    /// given: trimming it is the caller's decision.
    pub fn parse(&self, raw: &str) -> Result<Url, UrlRejection> {
        if let Some(limit) = self.max_bytes {
            if raw.len() > limit {
                return Err(UrlRejection::TooLong { limit });
            }
        }
        let url = Url::parse(raw).map_err(UrlRejection::Invalid)?;
        self.check(&url)?;
        Ok(url)
    }

    /// Checks an already parsed URL, such as one joined onto a base.
    pub fn check(&self, url: &Url) -> Result<(), UrlRejection> {
        if !self.schemes.contains(&url.scheme()) {
            return Err(UrlRejection::Scheme);
        }
        if url.host_str().is_none() {
            return Err(UrlRejection::MissingHost);
        }
        if !self.credentials && (!url.username().is_empty() || url.password().is_some()) {
            return Err(UrlRejection::Credentials);
        }
        if !self.query && url.query().is_some() {
            return Err(UrlRejection::Query);
        }
        if !self.fragment && url.fragment().is_some() {
            return Err(UrlRejection::Fragment);
        }
        Ok(())
    }
}

impl std::fmt::Display for UrlRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong { limit } => write!(formatter, "must not exceed {limit} bytes"),
            Self::Invalid(error) => write!(formatter, "must be a valid URL: {error}"),
            Self::Scheme => formatter.write_str("uses an unsupported scheme"),
            Self::MissingHost => formatter.write_str("must include a host"),
            Self::Credentials => formatter.write_str("must not contain credentials"),
            Self::Query => formatter.write_str("must not contain query parameters"),
            Self::Fragment => formatter.write_str("must not contain a fragment"),
        }
    }
}

impl std::error::Error for UrlRejection {}

#[cfg(test)]
mod tests;
