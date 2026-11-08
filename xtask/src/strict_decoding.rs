//! Keeps strict decoding where the input source asks for it
//! (AGENTS.md, "Decoding by input source").
//!
//! `#[serde(deny_unknown_fields)]` turns one field nobody expected into a
//! failure of the whole value. That is right for a request a host's frontend
//! sends over IPC, where drift should fail at once, and for an integrity
//! manifest we produce and check ourselves. It is wrong for our persisted
//! data (a newer or older build's file must still open), for what an
//! external producer sends and for model output. So every production type
//! carrying the attribute is listed, with its category and reason, in
//! `architecture/strict-decoding-policy.toml`; an unlisted type
//! fails, and so does an entry whose type no longer carries the attribute.
//!
//! The attribute is found in any spelling and position: inside any
//! `#[serde(...)]` or `#[cfg_attr(...)]` list on the type, its fields or its
//! variants. One written anywhere the check cannot tie to a type (another
//! item's attribute, a macro body) fails as it is.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use serde::Deserialize;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, Meta};

use crate::source_layout::package_roots;

const POLICY: &str = include_str!("../../architecture/strict-decoding-policy.toml");
const POLICY_FILE: &str = "architecture/strict-decoding-policy.toml";
const ATTRIBUTE: &str = "deny_unknown_fields";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StrictDecodingPolicy {
    version: u32,
    #[serde(default)]
    types: Vec<StrictType>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictType {
    /// Repository-relative path with `/` separators.
    path: String,
    /// The type's name, qualified by its inline modules (`module::Type`).
    #[serde(rename = "type")]
    type_name: String,
    category: Category,
    reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Category {
    /// Deserialized from a Tauri command argument our own frontend sends.
    IpcRequest,
    /// A manifest or handoff we produce and check by digest or signature.
    IntegrityManifest,
    /// Security-relevant strictness that is neither; the reason says why.
    Other,
}

impl fmt::Display for Category {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::IpcRequest => "ipc-request",
            Self::IntegrityManifest => "integrity-manifest",
            Self::Other => "other",
        })
    }
}

pub(crate) fn parse_policy(text: &str) -> Result<StrictDecodingPolicy, String> {
    let policy = toml::from_str::<StrictDecodingPolicy>(text)
        .map_err(|error| format!("parse {POLICY_FILE}: {error}"))?;
    if policy.version != 1 {
        return Err(format!(
            "unsupported strict decoding policy version {}",
            policy.version
        ));
    }
    Ok(policy)
}

/// One `deny_unknown_fields` found in a source file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Occurrence {
    pub(crate) line: usize,
    /// The type carrying it, or `None` where it cannot be tied to one.
    pub(crate) type_name: Option<String>,
}

fn is_test_source(name: &str) -> bool {
    matches!(
        name,
        "tests" | "tests.rs" | "test_support" | "test_support.rs" | "test_access.rs"
    )
}

fn production_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_test_source(&name) {
            continue;
        }
        if path.is_dir() {
            production_sources(&path, files);
        } else if name.ends_with(".rs") {
            files.push(path);
        }
    }
}

/// The production source roots: `src/` of every crate under `crates/`. The
/// xtask's own policy files are hand-edited and checked here, so it is not
/// one of them.
fn source_roots(root: &Path) -> Vec<PathBuf> {
    package_roots(root)
        .into_iter()
        .filter(|package| !package.ends_with("xtask"))
        .map(|package| package.join("src"))
        .collect()
}

fn relative_to(base: &Path, file: &Path) -> String {
    file.strip_prefix(base)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every `deny_unknown_fields` in `source`, in line order.
pub(crate) fn occurrences(source: &str) -> Result<Vec<Occurrence>, String> {
    let file = syn::parse_file(source).map_err(|error| error.to_string())?;
    let mut finder = Finder::default();
    finder.visit_file(&file);
    finder.found.sort_by_key(|occurrence| occurrence.line);
    Ok(finder.found)
}

#[derive(Default)]
struct Finder {
    modules: Vec<String>,
    found: Vec<Occurrence>,
}

impl Finder {
    /// Record `ident` when any of `attributes` (the type's, its fields' or
    /// its variants') names the attribute.
    fn record_type<'a>(
        &mut self,
        ident: &syn::Ident,
        attributes: impl IntoIterator<Item = &'a Attribute>,
    ) {
        let line = attributes
            .into_iter()
            .filter(|attribute| names_attribute(attribute))
            .map(|attribute| attribute.span().start().line)
            .min();
        if let Some(line) = line {
            let mut name = self.modules.clone();
            name.push(ident.to_string());
            self.found.push(Occurrence {
                line,
                type_name: Some(name.join("::")),
            });
        }
    }
}

fn field_attributes(fields: &syn::Fields) -> impl Iterator<Item = &Attribute> {
    fields.iter().flat_map(|field| field.attrs.iter())
}

impl<'ast> Visit<'ast> for Finder {
    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.record_type(
            &node.ident,
            node.attrs.iter().chain(field_attributes(&node.fields)),
        );
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        let variants = node.variants.iter().flat_map(|variant| {
            variant
                .attrs
                .iter()
                .chain(field_attributes(&variant.fields))
        });
        self.record_type(&node.ident, node.attrs.iter().chain(variants));
    }

    fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
        let fields = node
            .fields
            .named
            .iter()
            .flat_map(|field| field.attrs.iter());
        self.record_type(&node.ident, node.attrs.iter().chain(fields));
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.modules.push(node.ident.to_string());
        syn::visit::visit_item_mod(self, node);
        self.modules.pop();
    }

    /// An attribute outside a type: nothing it could make strict is listed.
    fn visit_attribute(&mut self, node: &'ast Attribute) {
        if names_attribute(node) {
            self.found.push(Occurrence {
                line: node.span().start().line,
                type_name: None,
            });
        }
    }

    /// Macro bodies are tokens: a type a macro defines is not seen as one.
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if let Some(line) = token_line(node.tokens.clone()) {
            self.found.push(Occurrence {
                line,
                type_name: None,
            });
        }
        syn::visit::visit_macro(self, node);
    }
}

/// Whether `attribute` is a `serde(...)` or `cfg_attr(...)` list naming the
/// attribute anywhere inside it.
fn names_attribute(attribute: &Attribute) -> bool {
    match &attribute.meta {
        Meta::List(list) if list.path.is_ident("serde") || list.path.is_ident("cfg_attr") => {
            token_line(list.tokens.clone()).is_some()
        }
        _ => false,
    }
}

/// The line of the first `deny_unknown_fields` identifier in `tokens`.
fn token_line(tokens: TokenStream) -> Option<usize> {
    tokens.into_iter().find_map(|token| match token {
        TokenTree::Ident(ident) if ident == ATTRIBUTE => Some(ident.span().start().line),
        TokenTree::Group(group) => token_line(group.stream()),
        _ => None,
    })
}

/// Every violation of `policy` by the production sources under `roots`,
/// with paths relative to `base`.
pub(crate) fn evaluate(
    base: &Path,
    roots: &[PathBuf],
    policy: &StrictDecodingPolicy,
) -> Vec<String> {
    let mut violations = Vec::new();
    let mut listed = BTreeMap::new();
    for entry in &policy.types {
        let key = (entry.path.as_str(), entry.type_name.as_str());
        if listed.insert(key, entry).is_some() {
            violations.push(format!(
                "{} `{}`: listed twice in {POLICY_FILE}.",
                entry.path, entry.type_name
            ));
        }
        if entry.reason.trim().is_empty() {
            violations.push(format!(
                "{} `{}`: its {POLICY_FILE} entry has no reason.",
                entry.path, entry.type_name
            ));
        }
    }

    let mut files = Vec::new();
    for root in roots {
        production_sources(root, &mut files);
    }
    files.sort();
    let mut seen = BTreeSet::new();
    for file in files {
        let path = relative_to(base, &file);
        let source = match fs::read_to_string(&file) {
            Ok(source) => source,
            Err(error) => {
                violations.push(format!("{path}: read: {error}"));
                continue;
            }
        };
        let found = match occurrences(&source) {
            Ok(found) => found,
            Err(error) => {
                violations.push(format!("{path}: parse: {error}"));
                continue;
            }
        };
        for occurrence in found {
            let Some(type_name) = occurrence.type_name else {
                violations.push(format!(
                    "{path}:{}: `{ATTRIBUTE}` outside a type definition the check can \
                     name (another item's attribute or a macro body); write it on the \
                     type itself so it can be listed.",
                    occurrence.line
                ));
                continue;
            };
            if listed.contains_key(&(path.as_str(), type_name.as_str())) {
                seen.insert((path.clone(), type_name));
            } else {
                violations.push(format!(
                    "{path}:{}: `{type_name}` uses `{ATTRIBUTE}` but is not listed in \
                     {POLICY_FILE}.",
                    occurrence.line
                ));
            }
        }
    }

    for entry in &policy.types {
        if !seen.contains(&(entry.path.clone(), entry.type_name.clone())) {
            violations.push(format!(
                "{} `{}` ({}): no longer a type with `{ATTRIBUTE}` in that file; \
                 remove its entry from {POLICY_FILE}.",
                entry.path, entry.type_name, entry.category
            ));
        }
    }
    violations
}

pub fn run(root: &Path) -> Result<(), String> {
    let policy = parse_policy(POLICY)?;
    let violations = evaluate(root, &source_roots(root), &policy);
    if violations.is_empty() {
        println!("Strict decoding check passed.");
        return Ok(());
    }
    Err(format!(
        "strict decoding violations:\n\n{}\n\n\
         Strictness follows the input source (AGENTS.md, \"Decoding by input \
         source\"). A request a host's frontend sends over IPC \
         and an integrity manifest we produce decode strictly and are listed in \
         {POLICY_FILE} with their reason. Our persisted data, external producers and \
         model output decode tolerantly: remove `{ATTRIBUTE}` there instead.\n",
        violations.join("\n")
    ))
}

#[cfg(test)]
mod tests;
