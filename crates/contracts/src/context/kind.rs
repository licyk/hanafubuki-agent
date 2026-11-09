use std::borrow::Cow;
use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize};

/// The kind of a context section: what one entry of the Agent's per-turn
/// world state describes.
///
/// An open identifier: a plugin names the kinds it contributes, and the
/// plugin that registers a kind's [`ContextRenderer`](super::ContextRenderer)
/// owns it. The wire form is the bare name, so recorded history keeps reading
/// back. Kinds compare by name; the model sees sections in the order of their
/// renderers' slots, never in kind order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ContextSectionKind(Cow<'static, str>);

impl ContextSectionKind {
    pub const WORKSPACE_INSTRUCTIONS: Self = Self::fixed("workspace_instructions");
    pub const WORKSPACE_ROOTS: Self = Self::fixed("workspace_roots");
    pub const PERMISSION_MODE: Self = Self::fixed("permission_mode");
    /// The sandbox of the turn's commands, and how one may leave it.
    pub const SANDBOX: Self = Self::fixed("sandbox");
    /// Written by the kernel for each sampling step; no contributor produces it.
    pub const TOOL_CATALOG_OMISSIONS: Self = Self::fixed("tool_catalog_omissions");
    pub const ENVIRONMENT: Self = Self::fixed("environment");

    /// The kinds of the Agent's own world state, which have their own places
    /// in a world state's bound and never use up what plugins (a host's
    /// among them) may add.
    pub const KNOWN: [Self; 6] = [
        Self::WORKSPACE_INSTRUCTIONS,
        Self::WORKSPACE_ROOTS,
        Self::PERMISSION_MODE,
        Self::SANDBOX,
        Self::TOOL_CATALOG_OMISSIONS,
        Self::ENVIRONMENT,
    ];

    pub const MAX_BYTES: usize = 64;

    /// A compile-time kind name; it must satisfy [`Self::is_valid`] (checked
    /// when a contributor or a renderer of the kind registers).
    pub const fn fixed(name: &'static str) -> Self {
        Self(Cow::Borrowed(name))
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::is_valid(name).then(|| Self(Cow::Owned(name.to_owned())))
    }

    /// Lower-case ASCII letters, digits and `_`, starting with a letter.
    pub fn is_valid(name: &str) -> bool {
        name.len() <= Self::MAX_BYTES
            && name
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContextSectionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A recorded kind that is not an identifier is invalid history, not an
/// unknown kind: it fails to read.
impl<'de> Deserialize<'de> for ContextSectionKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::parse(&name)
            .ok_or_else(|| de::Error::custom(format!("invalid context section kind {name:?}")))
    }
}
