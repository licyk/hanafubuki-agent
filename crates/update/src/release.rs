//! A release as GitHub's API describes it, read tolerantly: unknown fields
//! are ignored and an asset that does not read is left out, but the release
//! itself must be a published, stable `v<semver>`.

use std::net::IpAddr;

use reqwest::Url;
use semver::Version;
use serde::Deserialize;
use serde_json::Value;

use crate::error::UpdateError;
use crate::version::{release_version, TagProblem};

/// The releases, for a person to read.
pub const RELEASES_PAGE: &str = "https://github.com/licyk/hanafubuki-agent/releases";

/// The page of the release of `version`, built from the parsed version,
/// never taken from an answer.
pub fn release_notes_url(version: &Version) -> String {
    format!("{RELEASES_PAGE}/tag/v{version}")
}

/// The latest release: its version and the assets that read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub assets: Vec<ReleaseAsset>,
}

impl Release {
    /// The asset named exactly `name`.
    pub fn asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// One asset of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    /// `https`, or `http` to a loopback address from the test release URL.
    pub url: Url,
    /// In bytes, as GitHub gives it.
    pub size: u64,
    /// GitHub's `sha256:<hex>` digest; `None` when it gives none or another.
    pub sha256: Option<[u8; 32]>,
}

/// A text from the answer quoted in a message: escaped, and cut short.
const QUOTED_CHARS: usize = 40;

#[derive(Deserialize)]
struct RawRelease {
    tag_name: Option<String>,
    #[serde(default)]
    draft: Option<bool>,
    #[serde(default)]
    prerelease: Option<bool>,
    #[serde(default)]
    assets: Option<Vec<Value>>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

impl Release {
    /// The release in GitHub's answer `body`; `loopback_http` lets an asset
    /// be served over plain `http` from this machine (the test release URL).
    pub(crate) fn from_api_json(body: &[u8], loopback_http: bool) -> Result<Self, UpdateError> {
        let raw: RawRelease =
            serde_json::from_slice(body).map_err(|_| not_understood("it is not a release"))?;
        let tag = raw
            .tag_name
            .ok_or_else(|| not_understood("it names no tag"))?;
        if raw.draft.unwrap_or(false) {
            return Err(not_understood("it is a draft"));
        }
        if raw.prerelease.unwrap_or(false) {
            return Err(not_understood("it is a pre-release"));
        }
        let version = release_version(&tag).map_err(|problem| {
            let tag = quoted(&tag);
            not_understood(&match problem {
                TagProblem::NotAVersion => format!("the tag {tag} is not v<version>"),
                TagProblem::PreRelease => format!("the tag {tag} is a pre-release version"),
            })
        })?;
        let raw_assets = raw.assets.unwrap_or_default();
        let count = raw_assets.len();
        let assets: Vec<ReleaseAsset> = raw_assets
            .into_iter()
            .filter_map(|value| ReleaseAsset::from_value(value, loopback_http))
            .collect();
        if assets.len() < count {
            log::debug!(
                "release lookup: {} assets do not read and are left out",
                count - assets.len()
            );
        }
        Ok(Self { version, assets })
    }
}

impl ReleaseAsset {
    fn from_value(value: Value, loopback_http: bool) -> Option<Self> {
        let raw: RawAsset = serde_json::from_value(value).ok()?;
        let url = Url::parse(&raw.browser_download_url).ok()?;
        if !(url.scheme() == "https" || loopback_http && is_loopback_http(&url)) {
            return None;
        }
        Some(Self {
            name: raw.name,
            url,
            size: raw.size,
            sha256: raw.digest.as_deref().and_then(sha256_digest),
        })
    }
}

/// Whether `url` is plain `http` to this machine.
pub(crate) fn is_loopback_http(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    let host = host.trim_start_matches('[').trim_end_matches(']');
    url.scheme() == "http"
        && (host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback()))
}

/// `sha256:<64 hex digits>`.
fn sha256_digest(digest: &str) -> Option<[u8; 32]> {
    let hex = digest.strip_prefix("sha256:")?;
    let mut bytes = [0; 32];
    hex::decode_to_slice(hex, &mut bytes).ok()?;
    Some(bytes)
}

fn not_understood(reason: &str) -> UpdateError {
    UpdateError::NotUnderstood {
        reason: reason.to_string(),
    }
}

/// `text` in quotes with its control characters escaped, cut at
/// [`QUOTED_CHARS`].
fn quoted(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(QUOTED_CHARS).collect();
    let more = if chars.next().is_some() { "…" } else { "" };
    format!("{head:?}{more}")
}

#[cfg(test)]
mod tests;
