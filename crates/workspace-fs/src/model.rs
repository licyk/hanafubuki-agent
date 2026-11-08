use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedBinaryFile {
    pub canonical_path: String,
    pub bytes: Vec<u8>,
}

pub const MAX_PATH_BYTES: usize = 4 * 1024;
pub const MAX_LIST_ENTRIES: usize = 500;
pub const MAX_DIRECTORY_DEPTH: usize = 8;
pub const MIN_READ_PAGE_BYTES: usize = 4;
pub const MAX_READ_PAGE_BYTES: usize = 256 * 1024;
pub const MAX_READ_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_SEARCH_RESULTS: usize = 200;
pub const MAX_SEARCH_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_WRITE_BYTES: usize = 1024 * 1024;
pub const MAX_PATCH_BYTES: usize = 256 * 1024;
pub const MAX_PATCH_FILES: usize = 64;
pub const MAX_SEARCH_QUERY_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListDirectoryRequest {
    pub path: String,
    pub max_depth: usize,
    pub max_entries: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTextRequest {
    pub path: String,
    pub query: String,
    pub max_depth: usize,
    pub max_results: usize,
    pub max_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadFileRequest {
    pub path: String,
    pub offset: usize,
    pub max_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String,
    pub overwrite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyPatchRequest {
    pub patch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovePathRequest {
    pub source: String,
    pub destination: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletePathRequest {
    pub path: String,
    pub recursive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceFsRequest {
    ListDirectory(ListDirectoryRequest),
    SearchText(SearchTextRequest),
    ReadFile(ReadFileRequest),
    WriteFile(WriteFileRequest),
    ApplyPatch(ApplyPatchRequest),
    MovePath(MovePathRequest),
    DeletePath(DeletePathRequest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePathKind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub path: String,
    pub kind: WorkspacePathKind,
    pub size: u64,
    pub depth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryListing {
    pub root: String,
    pub entries: Vec<DirectoryEntry>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub root: String,
    pub matches: Vec<SearchMatch>,
    pub scanned_files: usize,
    pub scanned_bytes: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReadResult {
    pub path: String,
    pub content: String,
    pub sha256: String,
    pub offset: usize,
    pub next_offset: usize,
    pub bytes: usize,
    pub total_bytes: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMutationResult {
    pub path: String,
    pub kind: WorkspacePathKind,
    pub bytes: Option<usize>,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePathResult {
    pub source: String,
    pub destination: String,
    pub kind: WorkspacePathKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchMutationResult {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkspaceFsResult {
    DirectoryListing(DirectoryListing),
    SearchResults(SearchResults),
    FileRead(FileReadResult),
    FileWritten(FileMutationResult),
    PatchApplied(PatchMutationResult),
    PathMoved(MovePathResult),
    PathDeleted(FileMutationResult),
}

/// One patch target that resolved outside the workspace.
///
/// Both spellings are kept. The model wrote `requested` and can only correct
/// what it recognises; `resolved` is where that text actually landed, and is
/// what the person approving the operation needs to see.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalPathReport {
    pub requested: String,
    pub resolved: String,
}

impl fmt::Display for ExternalPathReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.requested == self.resolved {
            formatter.write_str(&self.resolved)
        } else {
            write!(
                formatter,
                "{} (written as {})",
                self.resolved, self.requested
            )
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceFsError {
    InvalidRequest(String),
    Resolution(String),
    /// The location does not exist. Kept apart from [`Self::Resolution`] so a
    /// caller that retries as a create cannot also swallow a permission error
    /// or a disconnected drive and report it to the model as "missing".
    PathMissing(String),
    ProtectedPath(String),
    ApprovalRequired,
    ApprovalMismatch,
    PathChanged,
    PreimageMismatch(String),
    BinaryFile,
    LimitExceeded(String),
    Cancelled,
    Io(String),
}

impl fmt::Display for WorkspaceFsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) => {
                write!(formatter, "invalid filesystem request: {message}")
            }
            Self::Resolution(message) => write!(formatter, "resolve filesystem target: {message}"),
            Self::PathMissing(path) => {
                write!(formatter, "filesystem target does not exist: {path}")
            }
            Self::ProtectedPath(path) => write!(
                formatter,
                "direct filesystem tools cannot access application-owned path {path}"
            ),
            Self::ApprovalRequired => {
                formatter.write_str("filesystem operation requires an exact consumed approval")
            }
            Self::ApprovalMismatch => formatter
                .write_str("filesystem approval does not match the current resolved operation"),
            Self::PathChanged => formatter.write_str("filesystem target changed after approval"),
            Self::PreimageMismatch(message) => {
                write!(
                    formatter,
                    "file preimage no longer matches the approved patch: {message}"
                )
            }
            Self::BinaryFile => {
                formatter.write_str("binary file content is not exposed to the model")
            }
            Self::LimitExceeded(message) => {
                write!(formatter, "filesystem limit exceeded: {message}")
            }
            Self::Cancelled => formatter.write_str("filesystem operation cancelled"),
            Self::Io(message) => write!(formatter, "filesystem operation failed: {message}"),
        }
    }
}

impl std::error::Error for WorkspaceFsError {}
