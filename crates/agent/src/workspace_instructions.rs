use std::io::Read;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::{PathError, PathKey, ResolvedPath};

use super::AgentWorkspaceRoots;

const AGENT_WORKSPACE_INSTRUCTIONS_TOTAL_BYTES: usize = 32 * 1024;
const AGENT_WORKSPACE_INSTRUCTIONS_PER_ROOT_BYTES: usize = 16 * 1024;
const AGENT_WORKSPACE_INSTRUCTION_FILES: usize = 32;
const AGENT_WORKSPACE_INSTRUCTION_READ_BYTES: usize = 64 * 1024;
const AGENT_WORKSPACE_INSTRUCTION_WARNING_BYTES: usize = 2 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentWorkspaceInstructionDiscovery {
    pub content: Option<String>,
    pub warnings: Vec<String>,
}

struct InstructionDocument {
    root: String,
    source: String,
    content: String,
}

/// The instructions of the user's own `AGENTS.md` (or `AGENTS.override.md`)
/// in `user_directory`, then those of each workspace root from the root down
/// to `selected_target`, all within one budget.
pub(crate) fn discover_workspace_instructions(
    user_directory: &Path,
    roots: &AgentWorkspaceRoots,
    selected_target: Option<&Path>,
) -> AgentWorkspaceInstructionDiscovery {
    discover_workspace_instructions_with_budget(
        user_directory,
        roots,
        selected_target,
        AGENT_WORKSPACE_INSTRUCTIONS_TOTAL_BYTES,
        AGENT_WORKSPACE_INSTRUCTIONS_PER_ROOT_BYTES,
    )
}

fn discover_workspace_instructions_with_budget(
    user_directory: &Path,
    roots: &AgentWorkspaceRoots,
    selected_target: Option<&Path>,
    total_budget: usize,
    per_root_budget: usize,
) -> AgentWorkspaceInstructionDiscovery {
    let mut discovery = AgentWorkspaceInstructionDiscovery::default();
    let selected = selected_target.and_then(|target| match ResolvedPath::resolve(target) {
        Ok(target) => Some(target),
        Err(error) => {
            push_warning(
                &mut discovery.warnings,
                format!("selected workspace target unavailable: {error}"),
            );
            None
        }
    });
    // The user's instructions are read first, so the project's more specific
    // ones follow them; each is a group with a fair share of the budget.
    let mut by_root = Vec::new();
    match ResolvedPath::resolve(user_directory) {
        Ok(directory) => by_root.push(
            read_directory_instruction(&directory, &directory, &mut discovery.warnings)
                .into_iter()
                .collect::<Vec<_>>(),
        ),
        Err(PathError::NotFound(_)) => {}
        Err(error) => push_warning(
            &mut discovery.warnings,
            format!("user instruction directory unavailable: {error}"),
        ),
    }
    for root_path in roots.roots() {
        if by_root.len() >= AGENT_WORKSPACE_INSTRUCTION_FILES {
            push_warning(
                &mut discovery.warnings,
                "workspace instruction root limit reached".to_string(),
            );
            break;
        }
        let root = match ResolvedPath::resolve(root_path) {
            Ok(root) => root,
            Err(error) => {
                push_warning(
                    &mut discovery.warnings,
                    format!("workspace root unavailable: {error}"),
                );
                continue;
            }
        };
        let target = selected
            .as_ref()
            .filter(|target| target.starts_with(&root))
            .unwrap_or(&root);
        let directories = hierarchy(&root, target, &mut discovery.warnings);
        let mut documents = Vec::new();
        for directory in directories {
            if documents.len() >= AGENT_WORKSPACE_INSTRUCTION_FILES {
                push_warning(
                    &mut discovery.warnings,
                    "workspace instruction file limit reached".to_string(),
                );
                break;
            }
            if let Some(document) =
                read_directory_instruction(&root, &directory, &mut discovery.warnings)
            {
                documents.push(document);
            }
        }
        by_root.push(documents);
    }

    let active_roots = by_root
        .iter()
        .filter(|documents| !documents.is_empty())
        .count();
    if active_roots == 0 || total_budget == 0 {
        return discovery;
    }
    let fair_share = total_budget.checked_div(active_roots).unwrap_or(0).max(1);
    let mut remaining_total = total_budget;
    let mut rendered = Vec::new();
    for documents in by_root {
        if documents.is_empty() || remaining_total == 0 {
            continue;
        }
        let mut remaining_root = per_root_budget.min(fair_share).min(remaining_total);
        for document in documents {
            if remaining_root == 0 {
                push_warning(
                    &mut discovery.warnings,
                    format!("workspace instructions truncated for {}", document.root),
                );
                break;
            }
            let header = format!(
                "# AGENTS.md instructions for {}\n\n<INSTRUCTIONS source=\"{}\">\n",
                document.root, document.source
            );
            let footer = "\n</INSTRUCTIONS>";
            if header.len().saturating_add(footer.len()) >= remaining_root {
                push_warning(
                    &mut discovery.warnings,
                    format!(
                        "workspace instruction metadata exceeded budget for {}",
                        document.root
                    ),
                );
                break;
            }
            let content_budget = remaining_root - header.len() - footer.len();
            let content = truncate_head_tail(&document.content, content_budget);
            let entry = format!("{header}{content}{footer}");
            remaining_root = remaining_root.saturating_sub(entry.len());
            remaining_total = remaining_total.saturating_sub(entry.len());
            rendered.push(entry);
        }
    }
    if !rendered.is_empty() {
        discovery.content = Some(rendered.join("\n\n"));
    }
    discovery
}

fn hierarchy(
    root: &ResolvedPath,
    target: &ResolvedPath,
    warnings: &mut Vec<String>,
) -> Vec<ResolvedPath> {
    if !target.starts_with(root) {
        push_warning(
            warnings,
            format!(
                "selected target {} is outside workspace root {}",
                target.native_text(),
                root.native_text()
            ),
        );
        return vec![root.clone()];
    }
    let mut paths = Vec::<PathBuf>::new();
    let mut current = Some(target.native());
    while let Some(path) = current {
        paths.push(path.to_path_buf());
        if PathKey::of(path) == *root.key() {
            break;
        }
        current = path.parent();
    }
    paths.reverse();
    paths
        .into_iter()
        .filter_map(|path| match ResolvedPath::resolve(&path) {
            Ok(path) => Some(path),
            Err(error) => {
                push_warning(
                    warnings,
                    format!("workspace directory unavailable: {error}"),
                );
                None
            }
        })
        .collect()
}

fn read_directory_instruction(
    root: &ResolvedPath,
    directory: &ResolvedPath,
    warnings: &mut Vec<String>,
) -> Option<InstructionDocument> {
    for name in ["AGENTS.override.md", "AGENTS.md"] {
        let candidate = match ResolvedPath::resolve_child(directory, name.as_ref()) {
            Ok(candidate) => candidate,
            Err(error) => {
                push_warning(
                    warnings,
                    format!("workspace instruction path failed: {error}"),
                );
                return None;
            }
        };
        let metadata = match std::fs::metadata(candidate.system()) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                push_warning(
                    warnings,
                    format!("cannot inspect {}: {error}", candidate.native_text()),
                );
                return None;
            }
        };
        if metadata.len() == 0 {
            continue;
        }
        let mut file = match std::fs::File::open(candidate.system()) {
            Ok(file) => file,
            Err(error) => {
                push_warning(
                    warnings,
                    format!("cannot read {}: {error}", candidate.native_text()),
                );
                return None;
            }
        };
        let mut bytes = Vec::new();
        if let Err(error) = file
            .by_ref()
            .take((AGENT_WORKSPACE_INSTRUCTION_READ_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
        {
            push_warning(
                warnings,
                format!("cannot read {}: {error}", candidate.native_text()),
            );
            return None;
        }
        if bytes.len() > AGENT_WORKSPACE_INSTRUCTION_READ_BYTES {
            bytes.truncate(AGENT_WORKSPACE_INSTRUCTION_READ_BYTES);
            push_warning(
                warnings,
                format!(
                    "workspace instruction read truncated: {}",
                    candidate.native_text()
                ),
            );
        }
        let invalid_utf8 = std::str::from_utf8(&bytes).is_err();
        let content = String::from_utf8_lossy(&bytes).into_owned();
        if invalid_utf8 {
            push_warning(
                warnings,
                format!(
                    "workspace instruction used lossy UTF-8: {}",
                    candidate.native_text()
                ),
            );
        }
        return Some(InstructionDocument {
            root: root.native_text().to_string(),
            source: candidate.native_text().to_string(),
            content,
        });
    }
    None
}

fn truncate_head_tail(value: &str, maximum: usize) -> String {
    const MARKER: &str = "\n... workspace instructions omitted ...\n";
    if value.len() <= maximum {
        return value.to_string();
    }
    if maximum <= MARKER.len() {
        return value[..value.floor_char_boundary(maximum)].to_string();
    }
    let available = maximum - MARKER.len();
    let mut head = available / 2;
    while !value.is_char_boundary(head) {
        head = head.saturating_sub(1);
    }
    let mut tail = value.len().saturating_sub(available - head);
    while tail < value.len() && !value.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}{}{}", &value[..head], MARKER, &value[tail..])
}

fn push_warning(warnings: &mut Vec<String>, warning: String) {
    if warnings.len() >= 16 {
        return;
    }
    warnings.push(truncate_head_tail(
        &warning,
        AGENT_WORKSPACE_INSTRUCTION_WARNING_BYTES,
    ));
}

#[cfg(test)]
mod tests;
