use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_SUMMARY_BYTES: usize = 2 * 1024;
const MAX_OPERATION_ID_BYTES: usize = 512;

macro_rules! typed_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub(crate) const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn value(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

typed_id!(ApprovalRequestId);
typed_id!(ApprovalGrantId);

macro_rules! digest_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn from_canonical_bytes(bytes: &[u8]) -> Self {
                let digest = Sha256::digest(bytes);
                let mut encoded = String::with_capacity(digest.len() * 2);
                for byte in digest {
                    use fmt::Write;
                    write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
                }
                Self(encoded)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

digest_type!(ApprovalActionDigest);
digest_type!(ApprovalContextDigest);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalActionKind {
    DomainMutation,
    CommandExecution,
    ShellExecution,
    CodeExecution,
    FileRead,
    FileWrite,
    FilePatch,
    FileMove,
    FileDelete,
    InteractiveExecution,
    SkillManagement,
    McpToolCall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRiskClass {
    Low,
    Medium,
    High,
    Critical,
}

impl ApprovalRiskClass {
    pub const fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
            Self::Critical => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ApprovalOrigin {
    User,
    Model,
    /// A script a Skill supplied. `content_digest` binds the approval to the
    /// exact bytes that will run.
    Skill {
        skill: String,
        content_digest: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAction {
    pub kind: ApprovalActionKind,
    pub risk: ApprovalRiskClass,
    pub origin: ApprovalOrigin,
    pub summary: String,
    pub digest: ApprovalActionDigest,
    pub details: Option<PermissionActionDetails>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PermissionActionDetails {
    Command {
        program: String,
        argv: Vec<String>,
        cwd: String,
    },
    Paths {
        canonical_paths: Vec<String>,
    },
    Domain {
        key: String,
    },
    SkillManagement {
        operation: String,
        /// The Skill this turns on or off.
        skill: Option<String>,
        effective_from: String,
    },
    McpTool {
        server_id: String,
        connection_generation: String,
        catalog_generation: String,
        tool_id: String,
        tool_generation: String,
        arguments_digest: String,
    },
}

impl ApprovalAction {
    pub fn from_canonical_bytes(
        kind: ApprovalActionKind,
        risk: ApprovalRiskClass,
        origin: ApprovalOrigin,
        summary: String,
        canonical: &[u8],
    ) -> Result<Self, ApprovalError> {
        if summary.trim().is_empty() || summary.len() > MAX_SUMMARY_BYTES {
            return Err(ApprovalError::InvalidAction(
                "approval summary must be non-empty and bounded".to_string(),
            ));
        }
        Ok(Self {
            kind,
            risk,
            origin,
            summary,
            digest: ApprovalActionDigest::from_canonical_bytes(canonical),
            details: None,
        })
    }

    pub fn with_details(mut self, target: PermissionActionDetails) -> Result<Self, ApprovalError> {
        target.validate()?;
        self.details = Some(target);
        Ok(self)
    }

    pub fn details(&self) -> Option<&PermissionActionDetails> {
        self.details.as_ref()
    }
}

impl PermissionActionDetails {
    fn validate(&self) -> Result<(), ApprovalError> {
        const MAX_PARTS: usize = 128;
        const MAX_PART_BYTES: usize = 16 * 1024;
        let valid = |value: &str| {
            !value.is_empty() && value.len() <= MAX_PART_BYTES && !value.as_bytes().contains(&0)
        };
        match self {
            Self::Command { program, argv, cwd }
                if !valid(program)
                    || !valid(cwd)
                    || argv.is_empty()
                    || argv.len() > MAX_PARTS
                    || argv.iter().any(|value| !valid(value)) =>
            {
                Err(ApprovalError::InvalidAction(
                    "command policy target is invalid or unbounded".to_string(),
                ))
            }
            Self::Paths { canonical_paths }
                if canonical_paths.is_empty()
                    || canonical_paths.len() > MAX_PARTS
                    || canonical_paths.iter().any(|value| !valid(value)) =>
            {
                Err(ApprovalError::InvalidAction(
                    "path policy target is invalid or unbounded".to_string(),
                ))
            }
            Self::Domain { key } if !valid(key) => Err(ApprovalError::InvalidAction(
                "domain policy target is invalid or unbounded".to_string(),
            )),
            Self::SkillManagement {
                operation,
                skill,
                effective_from,
            } if !valid(operation)
                || !valid(effective_from)
                || skill.as_deref().is_some_and(|value| !valid(value)) =>
            {
                Err(ApprovalError::InvalidAction(
                    "Skill management policy target is invalid or unbounded".to_string(),
                ))
            }
            Self::McpTool {
                server_id,
                connection_generation,
                catalog_generation,
                tool_id,
                tool_generation,
                arguments_digest,
            } if [
                server_id,
                connection_generation,
                catalog_generation,
                tool_id,
                tool_generation,
                arguments_digest,
            ]
            .into_iter()
            .any(|value| !valid(value)) =>
            {
                Err(ApprovalError::InvalidAction(
                    "MCP tool policy target is invalid or unbounded".to_string(),
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalContextScope {
    pub authority: String,
    pub session_id: String,
    pub operation_group_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalContext {
    operation_id: String,
    digest: ApprovalContextDigest,
    scope: Option<ApprovalContextScope>,
}

impl ApprovalContext {
    pub fn from_canonical_bytes(
        operation_id: String,
        canonical: &[u8],
    ) -> Result<Self, ApprovalError> {
        if operation_id.trim().is_empty() || operation_id.len() > MAX_OPERATION_ID_BYTES {
            return Err(ApprovalError::InvalidContext(
                "approval operation id must be non-empty and bounded".to_string(),
            ));
        }
        Ok(Self {
            operation_id,
            digest: ApprovalContextDigest::from_canonical_bytes(canonical),
            scope: None,
        })
    }

    pub fn with_scope(mut self, scope: ApprovalContextScope) -> Result<Self, ApprovalError> {
        for value in [
            &scope.authority,
            &scope.session_id,
            &scope.operation_group_id,
        ] {
            if value.trim().is_empty() || value.len() > 256 || value.as_bytes().contains(&0) {
                return Err(ApprovalError::InvalidContext(
                    "approval scope fields must be non-empty and bounded".to_string(),
                ));
            }
        }
        if scope.generation == 0 {
            return Err(ApprovalError::InvalidContext(
                "approval scope generation must be non-zero".to_string(),
            ));
        }
        let scoped_digest = [
            self.digest.as_str().as_bytes(),
            scope.authority.as_bytes(),
            scope.session_id.as_bytes(),
            scope.operation_group_id.as_bytes(),
            &scope.generation.to_be_bytes(),
        ]
        .into_iter()
        .fold(Vec::new(), |mut canonical, part| {
            canonical.extend_from_slice(&(part.len() as u64).to_be_bytes());
            canonical.extend_from_slice(part);
            canonical
        });
        self.digest = ApprovalContextDigest::from_canonical_bytes(&scoped_digest);
        self.scope = Some(scope);
        Ok(self)
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub fn digest(&self) -> &ApprovalContextDigest {
        &self.digest
    }

    pub fn scope(&self) -> Option<&ApprovalContextScope> {
        self.scope.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalUserDecision {
    AllowOnce,
    /// Allows this action and, for the rest of the session, every later use
    /// of the same tool that only the permission mode would ask about. Only a
    /// request that offers it accepts it
    /// (`ApprovalRequest::offers_allow_for_session`).
    AllowForSession,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowForSession,
    Deny,
    Aborted,
    TimedOut,
}

impl From<ApprovalUserDecision> for ApprovalDecision {
    fn from(value: ApprovalUserDecision) -> Self {
        match value {
            ApprovalUserDecision::AllowOnce => Self::AllowOnce,
            ApprovalUserDecision::AllowForSession => Self::AllowForSession,
            ApprovalUserDecision::Deny => Self::Deny,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionSource {
    AutoReviewer,
    FullAccess,
    User,
    /// The user's earlier `AllowForSession` of the same tool.
    SessionAllowance,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub request_id: ApprovalRequestId,
    pub operation_id: String,
    pub action: ApprovalAction,
    pub requested_at_ms: u128,
    pub expires_at_ms: u128,
    /// Whether the user may answer `AllowForSession`.
    pub offers_allow_for_session: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolution {
    pub request_id: ApprovalRequestId,
    pub decision: ApprovalDecision,
    pub source: ApprovalDecisionSource,
    pub resolved_at_ms: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalError {
    InvalidAction(String),
    InvalidContext(String),
    InvalidTimeout,
    RequestNotFound,
    StaleContext,
    GrantNotFound,
    GrantMismatch,
    GrantExpired,
    InvalidDecisionSource,
    DecisionNotOffered,
}

impl fmt::Display for ApprovalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAction(message) => write!(formatter, "invalid approval action: {message}"),
            Self::InvalidContext(message) => {
                write!(formatter, "invalid approval context: {message}")
            }
            Self::InvalidTimeout => formatter.write_str("approval timeout must be non-zero"),
            Self::RequestNotFound => formatter.write_str("approval request not found"),
            Self::StaleContext => formatter.write_str("stale approval context"),
            Self::GrantNotFound => formatter.write_str("approval grant not found or consumed"),
            Self::GrantMismatch => formatter.write_str("approval grant does not match the action"),
            Self::GrantExpired => formatter.write_str("approval grant expired"),
            Self::InvalidDecisionSource => {
                formatter.write_str("permission decision source cannot issue this authorization")
            }
            Self::DecisionNotOffered => {
                formatter.write_str("the approval request does not offer this decision")
            }
        }
    }
}

impl std::error::Error for ApprovalError {}
