//! Tauri-free authority for Agent Skills.
//!
//! A Skill is a directory containing a `SKILL.md` whose YAML frontmatter names
//! it and describes when to use it. That is the whole contract, and it is the
//! same one Claude and Codex implement, so a package that installs there
//! installs here. Everything beside `SKILL.md` — an `agents/openai.yaml`
//! interface, references, scripts, assets, a licence, a readme — is optional
//! decoration: it is read when it parses and ignored when it does not, and it
//! can never make an otherwise valid Skill fail to load.
//!
//! This crate discovers Skills under ordered roots, records which of them the
//! user turned off, reads their documents and resources, and owns the managed
//! user root where Skills are installed, authored, and deleted. It deliberately
//! does not execute scripts, dispatch tools, decide approvals, or own process
//! and task state.

mod discovery;
mod fs_util;
mod interface;
mod model;
mod parser;
mod service;
mod state;
mod system;
mod yaml_fields;

pub use discovery::repository_skill_roots;
pub use model::{
    SkillAssetRead, SkillAvailability, SkillAvailabilityState, SkillCatalogEntry,
    SkillCatalogListRequest, SkillCatalogSnapshot, SkillCreateRequest, SkillDeleteFileRequest,
    SkillDuplicateRequest, SkillError, SkillFileRead, SkillId, SkillInstallRequest, SkillInterface,
    SkillIssueSeverity, SkillLoadIssue, SkillMcpTransport, SkillRead, SkillResourceDescriptor,
    SkillResourceKind, SkillResourceRead, SkillRoots, SkillScope, SkillSourceInspection,
    SkillToolDependency, SkillTurnCatalog, SkillTurnEntry, SkillTurnUnavailable,
    SkillWriteFileRequest, MAX_SKILL_FILES, MAX_SKILL_FILE_BYTES,
};
pub use parser::slug_from_name;
pub use service::{SkillService, MAX_TURN_DOCUMENT_BYTES};
