//! Keeps context section wrappers out of `hanafubuki-agent-model-api`
//! (docs/rules/plugins.md, Context renderers).
//!
//! A context section's framing belongs to the plugin that registers its
//! kind's `ContextRenderer`; model-api sends a `ContextFragment`'s role and
//! rendered text as given and treats its `kind` as a label. So model-api's
//! production sources name no context section kind: neither as a string
//! literal (`"agent_memory"`, a `match` arm on the label) nor as the
//! identifier an enum of kinds would give it (`AgentMemory`, the form the
//! model-api-owned `ModelContextKind` had). The kinds are collected from
//! where they are declared, `ContextSectionKind`'s constants and every
//! `ContextSectionKind::fixed` in production code, so a plugin's new kind is
//! covered without editing this check. Model-api still frames its own items,
//! continuation context and compaction summaries, which are not sections.
//! That it cannot reach the kind type at all is the dependency check's
//! (`check-dependencies`: no `hanafubuki-agent-contracts`).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::locks::{checked_source_roots, production_sources};
use crate::source_layout::relative;

const MODEL_API: &str = "crates/model-api/src";
const KIND_CONSTANTS: &str = "crates/contracts/src/context/kind.rs";

pub fn run(root: &Path) -> Result<(), String> {
    let mut kinds = declared_kinds(
        &fs::read_to_string(root.join(KIND_CONSTANTS))
            .unwrap_or_else(|error| panic!("read {KIND_CONSTANTS}: {error}")),
        "Self::fixed(\"",
    );
    let model_api = root.join(MODEL_API);
    for source_root in checked_source_roots(root) {
        if source_root == model_api {
            continue;
        }
        for file in production_sources(&source_root) {
            let source = fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
            kinds.extend(declared_kinds(&source, "ContextSectionKind::fixed(\""));
        }
    }
    for expected in ["workspace_instructions", "skill_catalog", "agent_memory"] {
        if !kinds.contains(expected) {
            return Err(format!(
                "the context section kind {expected} was not found where kinds are declared; update xtask model_api_context"
            ));
        }
    }

    let mut reports = Vec::new();
    for file in production_sources(&model_api) {
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        for (line, found) in kind_mentions(&source, &kinds) {
            reports.push(format!("{}:{line}: {found}", relative(root, &file)));
        }
    }
    if reports.is_empty() {
        println!("Model API context section check passed.");
        return Ok(());
    }
    Err(format!(
        "hanafubuki-agent-model-api names context section kinds:\n\n{}\n\n\
         A section's wrapper text belongs to the plugin that registers its\n\
         ContextRenderer; model-api sends role and content as given\n\
         (docs/rules/plugins.md, Context renderers).\n",
        reports.join("\n")
    ))
}

/// The kind names passed to `marker` (a call opening a string literal) in
/// `source`.
pub(crate) fn declared_kinds(source: &str, marker: &str) -> BTreeSet<String> {
    source
        .match_indices(marker)
        .filter_map(|(start, _)| {
            let rest = &source[start + marker.len()..];
            Some(rest[..rest.find('"')?].to_owned())
        })
        .collect()
}

/// Every line of `source` naming one of `kinds`, as (1-based line, what was
/// found): the kind as a whole string literal, or, for a kind of more than one
/// word, its UpperCamelCase form as an identifier. A one-word kind's
/// identifier (`Environment`) is an ordinary word, so only its literal counts.
pub(crate) fn kind_mentions(source: &str, kinds: &BTreeSet<String>) -> Vec<(usize, String)> {
    let forms = kinds
        .iter()
        .map(|kind| {
            let identifier = kind.contains('_').then(|| upper_camel_case(kind));
            (format!("\"{kind}\""), identifier)
        })
        .collect::<Vec<_>>();
    let mut found = Vec::new();
    for (index, line) in source.lines().enumerate() {
        for (literal, identifier) in &forms {
            if line.contains(literal.as_str()) {
                found.push((index + 1, literal.clone()));
            }
            if let Some(identifier) = identifier {
                if line
                    .split(|character: char| {
                        !(character.is_ascii_alphanumeric() || character == '_')
                    })
                    .any(|token| token == identifier)
                {
                    found.push((index + 1, identifier.clone()));
                }
            }
        }
    }
    found
}

fn upper_camel_case(kind: &str) -> String {
    kind.split('_')
        .map(|word| {
            let mut characters = word.chars();
            characters
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
                .unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod tests;
