//! Tauri-free direct filesystem tools for exact, approved Agent operations.
//!
//! The crate resolves paths and content into immutable approval plans, then
//! re-resolves and verifies the same plan immediately before bounded I/O. It
//! does not own Config, instance lifecycle, Agent history, or approval policy.

mod atomic_write;
mod model;
mod patch;
mod patch_preview;
mod policy;
mod resolver;
mod service;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use model::{
    ApplyPatchRequest, DeletePathRequest, DirectoryEntry, DirectoryListing, ExternalPathReport,
    FileMutationResult, FileReadResult, ListDirectoryRequest, MovePathRequest, MovePathResult,
    PatchHunkStart, PatchMutationResult, ReadFileRequest, SearchMatch, SearchResults,
    SearchTextRequest, SelectedBinaryFile, WorkspaceFsError, WorkspaceFsRequest, WorkspaceFsResult,
    WorkspacePathKind, WriteFileRequest, MAX_DIRECTORY_DEPTH, MAX_LIST_ENTRIES, MAX_PATCH_BYTES,
    MAX_PATCH_FILES, MAX_PATH_BYTES, MAX_READ_FILE_BYTES, MAX_READ_PAGE_BYTES, MAX_SEARCH_BYTES,
    MAX_SEARCH_FILE_BYTES, MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_RESULTS, MAX_WRITE_BYTES,
    MIN_READ_PAGE_BYTES,
};
pub use patch_preview::{
    preview_patch, PatchPreview, PatchPreviewFile, PatchPreviewLine, PatchPreviewLineKind,
    PatchPreviewOperation,
};
pub use policy::WorkspaceFsPolicy;
pub use resolver::PreparedWorkspaceOperation;
pub use service::WorkspaceFsService;
