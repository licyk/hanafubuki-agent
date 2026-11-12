// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The headings the command center groups its sessions under (Codex's
//! `agents_overview_grouping.rs` and `AgentsOverviewProjectGroup`): the
//! project, the directory, the status or the model, cycled with `g` and
//! kept in `tui.agentsGrouping`.
//!
//! A project is the directory within the repository's main checkout, so a
//! session in one of the repository's worktrees (`/worktree`, `w`) falls
//! under the checkout it was made from; outside a repository it is the
//! directory. It is read from the `.git` file and the Git directory's
//! `commondir`, once for each directory: no process is started.

use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent::{
    normalize_path, relative_below, AgentAgentsGrouping, AgentSessionSummary, PathKey,
};

use super::status::{Group, Status};
use crate::appearance::display_directory;

/// What the heading of a session's directory is under the project and the
/// directory groupings.
#[derive(Clone, Debug)]
pub(crate) struct Place {
    project: (PathKey, String),
    directory: (PathKey, String),
}

impl Place {
    pub(crate) fn of(cwd: &str) -> Self {
        let directory = Path::new(cwd);
        let project = project(directory);
        Self {
            project: (
                PathKey::of(&project),
                display_directory(&project.display().to_string()),
            ),
            directory: (PathKey::of(directory), display_directory(cwd)),
        }
    }

    #[cfg(test)]
    pub(crate) fn project_key(&self) -> &PathKey {
        &self.project.0
    }

    #[cfg(test)]
    pub(crate) fn directory_key(&self) -> &PathKey {
        &self.directory.0
    }
}

/// What a heading is told apart by; its order is the headings' order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum HeadingKey {
    Status(Group),
    Path(PathKey),
    Model(String),
    /// A session that recorded no directory, under last.
    NoDirectory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Heading {
    pub(crate) key: HeadingKey,
    pub(crate) label: String,
}

impl Heading {
    /// The heading `summary` (with `status`) falls under; `place` is its
    /// directory's.
    pub(crate) fn of(
        grouping: AgentAgentsGrouping,
        summary: &AgentSessionSummary,
        status: Status,
        place: Option<&Place>,
    ) -> Self {
        let path = |(key, label): &(PathKey, String)| Self {
            key: HeadingKey::Path(key.clone()),
            label: label.clone(),
        };
        let no_directory = || Self {
            key: HeadingKey::NoDirectory,
            label: "No directory".to_string(),
        };
        match grouping {
            AgentAgentsGrouping::Status => Self {
                key: HeadingKey::Status(status.group),
                label: status.group.label().to_string(),
            },
            AgentAgentsGrouping::Model => {
                let model = Some(summary.model.as_str())
                    .filter(|model| !model.is_empty())
                    .unwrap_or("Unknown");
                Self {
                    key: HeadingKey::Model(model.to_string()),
                    label: model.to_string(),
                }
            }
            AgentAgentsGrouping::Project => {
                place.map_or_else(no_directory, |place| path(&place.project))
            }
            AgentAgentsGrouping::Directory => {
                place.map_or_else(no_directory, |place| path(&place.directory))
            }
        }
    }
}

/// The grouping `g` moves to.
pub(crate) fn next(grouping: AgentAgentsGrouping) -> AgentAgentsGrouping {
    match grouping {
        AgentAgentsGrouping::Project => AgentAgentsGrouping::Directory,
        AgentAgentsGrouping::Directory => AgentAgentsGrouping::Status,
        AgentAgentsGrouping::Status => AgentAgentsGrouping::Model,
        AgentAgentsGrouping::Model => AgentAgentsGrouping::Project,
    }
}

pub(crate) fn label(grouping: AgentAgentsGrouping) -> &'static str {
    match grouping {
        AgentAgentsGrouping::Project => "Project",
        AgentAgentsGrouping::Directory => "Directory",
        AgentAgentsGrouping::Status => "Status",
        AgentAgentsGrouping::Model => "Model",
    }
}

/// The project `cwd` is in: within a worktree's checkout, the same
/// directory within the repository's main checkout; else `cwd` itself.
fn project(cwd: &Path) -> PathBuf {
    for root in cwd.ancestors() {
        let dot_git = root.join(".git");
        if dot_git.is_dir() {
            break;
        }
        if dot_git.is_file() {
            let main = main_checkout(root, &dot_git)
                .and_then(|main| Some(main.join(relative_below(cwd, root)?)));
            return main.unwrap_or_else(|| cwd.to_path_buf());
        }
    }
    cwd.to_path_buf()
}

/// The main checkout of the worktree rooted at `root`: its `.git` file
/// names the worktree's Git directory, whose `commondir` names the main
/// checkout's `.git`. A submodule's Git directory has no `commondir`.
fn main_checkout(root: &Path, dot_git: &Path) -> Option<PathBuf> {
    let pointer = fs::read_to_string(dot_git).ok()?;
    let git_dir = root.join(pointer.trim().strip_prefix("gitdir:")?.trim());
    let common = fs::read_to_string(git_dir.join("commondir")).ok()?;
    let common = normalize_path(git_dir.join(common.trim())).ok()?;
    if common.file_name()? != ".git" {
        return None;
    }
    common.parent().map(Path::to_path_buf)
}
