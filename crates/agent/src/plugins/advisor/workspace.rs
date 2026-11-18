use super::{history::Evidence, AdvisorOptions};
use crate::conversation_reader::AgentConversationReader;
use hanafubuki_agent_contracts::CancellationToken;
use hanafubuki_agent_kernel::{AgentSessionId, ModelImageDetail, ModelUserContent};
use hanafubuki_agent_plugin_hooks::WorkingDirectory;
use hanafubuki_agent_workspace_fs::WorkspaceFsService;
use hanafubuki_agent_worktree::GitQueries;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum GitContext {
    #[default]
    None,
    Summary,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisorTrigger {
    #[default]
    Manual,
    ExecutorRequested,
    RepeatedToolCall,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Consultation {
    pub question: String,
    pub follow_up_to: Option<String>,
    pub draft: Option<String>,
    pub git_context: Option<GitContext>,
    pub include_tracked_files: Vec<String>,
    pub include_untracked: Vec<String>,
    #[serde(skip)]
    pub(super) root: Option<PathBuf>,
    #[serde(skip)]
    pub(super) trigger: AdvisorTrigger,
}

#[derive(Clone)]
pub(super) struct AdvisorWorkspace {
    pub git: GitQueries,
    pub files: WorkspaceFsService,
    pub directory: WorkingDirectory,
}

impl AdvisorWorkspace {
    pub(super) fn contribute(
        &self,
        id: AgentSessionId,
        opts: &AdvisorOptions,
        request: &Consultation,
        reader: &AgentConversationReader,
        evidence: &mut Evidence,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let level = request
            .git_context
            .unwrap_or(opts.git_context)
            .min(opts.git_context);
        if level == GitContext::None
            && !opts.project_rules
            && request.include_tracked_files.is_empty()
            && request.include_untracked.is_empty()
        {
            return Ok(());
        }
        let root = match &request.root {
            Some(root) => root.clone(),
            None => (self.directory)(id)?,
        };
        let files = self
            .files
            .with_workspace_roots(vec![root.clone()])
            .map_err(|e| e.to_string())?;
        let resolved_root = hanafubuki_agent_foundation::paths::ResolvedPath::resolve(&root)
            .map_err(|e| e.to_string())?;
        let read = |name: &str, budget| {
            let path = hanafubuki_agent_foundation::paths::ResolvedPath::resolve(root.join(name))
                .map_err(|e| e.to_string())?;
            if !path.starts_with(&resolved_root) {
                return Err(format!("Outside Advisor workspace: {name}"));
            }
            let file = match files.read_bounded_binary(name, budget, cancel) {
                Ok(file) => file,
                Err(hanafubuki_agent_workspace_fs::WorkspaceFsError::LimitExceeded(_)) => {
                    return Ok(None)
                }
                Err(error) => return Err(error.to_string()),
            };
            if !hanafubuki_agent_foundation::paths::PathKey::of(Path::new(&file.canonical_path))
                .starts_with(resolved_root.key())
            {
                return Err(format!("Outside Advisor workspace: {name}"));
            }
            Ok(Some(file.bytes))
        };
        if level != GitContext::None {
            let git = self
                .git
                .workspace_changes(
                    &root,
                    level == GitContext::Full,
                    &files
                        .repository_exclusions(&resolved_root)
                        .map_err(|e| e.to_string())?,
                )
                .unwrap_or_else(|e| format!("Git evidence unavailable: {e}"));
            let limit = opts.git_chars.min(opts.history_chars / 2);
            let git = opts.outbound(git);
            evidence.text.push_str(&format!(
                "\nWorkspace changes (untrusted):\n{}",
                git.chars().take(limit).collect::<String>()
            ));
        }
        if opts.project_rules && reader.trusts_project(&root)? {
            for (label, names) in [
                ("Workspace rules", &["AGENTS.override.md", "AGENTS.md"][..]),
                ("Project preferences", &[".advisor-preferences.md"][..]),
            ] {
                for name in names {
                    if let Ok(Some(bytes)) = read(name, 16384) {
                        if let Ok(text) = String::from_utf8(bytes) {
                            evidence.text.push_str(&format!(
                                "\n{label} from {name} (untrusted):\n{}",
                                opts.outbound(text)
                            ));
                            break;
                        }
                    }
                }
            }
        }
        let mut remaining = 24 * 1024;
        for (tracked, paths) in [
            (true, &request.include_tracked_files),
            (false, &request.include_untracked),
        ] {
            for path in paths {
                if !(if tracked {
                    opts.tracked_files
                } else {
                    opts.untracked_files
                }) {
                    return Err(
                        "Advisor file disclosure is disabled for the requested file class".into(),
                    );
                }
                if path.is_empty()
                    || !Path::new(path)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_)))
                    || !self
                        .git
                        .listed_file(&root, path, tracked)
                        .map_err(|e| e.to_string())?
                {
                    return Err(format!("No permitted repository file named {path}"));
                }
                // A Git entry can itself be a symlink; following it could disclose a different file.
                if root
                    .join(path)
                    .symlink_metadata()
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
                {
                    return Err(format!("Cannot attach symlink {path}"));
                }
                let image = Path::new(path)
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| {
                        matches!(
                            s.to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "gif" | "webp"
                        )
                    });
                let omitted = format!(
                    "\n[Attachment {} omitted: size or count budget]",
                    serde_json::json!(path)
                );
                if image {
                    if !opts.share_images {
                        evidence.text.push_str("\n[Image withheld]");
                        continue;
                    }
                    if evidence.images.len() + evidence.file_images.len() >= 4 {
                        evidence.text.push_str(&omitted);
                        continue;
                    }
                    let Some(bytes) = read(path, 4 * 1024 * 1024)? else {
                        evidence.text.push_str(&omitted);
                        continue;
                    };
                    let image = crate::attachments::normalize_image(
                        bytes,
                        path.clone(),
                        hanafubuki_agent_kernel::AgentImageDetail::Auto,
                    )?;
                    let total = evidence.images.iter().map(|i| i.byte_length).sum::<u64>()
                        + evidence
                            .file_images
                            .iter()
                            .map(|i| match i {
                                ModelUserContent::InputImage { data, .. } => data.len() as u64,
                                _ => 0,
                            })
                            .sum::<u64>();
                    if total + image.bytes.len() as u64 <= 8 * 1024 * 1024 {
                        evidence.file_images.push(ModelUserContent::InputImage {
                            mime_type: image.mime_type,
                            data: image.bytes,
                            detail: ModelImageDetail::Auto,
                        });
                    } else {
                        evidence.text.push_str(&omitted);
                    }
                } else {
                    if remaining == 0 {
                        evidence.text.push_str(&omitted);
                        continue;
                    }
                    let Some(bytes) = read(path, remaining.min(8192))? else {
                        evidence.text.push_str(&omitted);
                        continue;
                    };
                    remaining -= bytes.len();
                    let text =
                        String::from_utf8(bytes).map_err(|_| format!("File {path} is not text"))?;
                    evidence.text.push_str(&format!(
                        "\nNamed file {} (untrusted):\n{}",
                        serde_json::json!(path),
                        opts.outbound(text)
                    ));
                }
            }
        }
        Ok(())
    }
}
