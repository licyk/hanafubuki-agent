//! Keeps every lock a contained panic can reach non-poisoning (AGENTS.md,
//! "Locks").
//!
//! A `std::sync` lock is poisoned when its holder panics, and every later
//! `.lock().unwrap()` on it panics too: one fault in one call would fail every
//! call after it, and a drop guard taking the lock while unwinding would abort
//! the process. Panics unwind on contained threads, which run the Agent and
//! its plugins, so the production sources of every crate under `crates/` use
//! `parking_lot`, whose locks are released on unwind without poisoning, and
//! name no `std::sync::Mutex` or `std::sync::RwLock`, in any import form;
//! tests and test support may.

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{ItemUse, Macro, UseTree};

use crate::source_layout::relative;

/// The std lock types production code may not use. A name matches by prefix,
/// so the guard types (`MutexGuard`, `RwLockReadGuard`, ...) match too.
const BANNED: [&str; 2] = ["Mutex", "RwLock"];

/// The checked production source roots: the `src/` of every crate under
/// `crates/`.
pub(crate) fn checked_source_roots(root: &Path) -> Vec<PathBuf> {
    let crates = root.join("crates");
    let mut roots = fs::read_dir(&crates)
        .unwrap_or_else(|error| panic!("read {}: {error}", crates.display()))
        .flatten()
        .map(|entry| entry.path().join("src"))
        .filter(|source| source.is_dir())
        .collect::<Vec<_>>();
    roots.sort();
    roots
}

fn is_test_source(name: &str) -> bool {
    matches!(
        name,
        "tests" | "tests.rs" | "test_support" | "test_support.rs"
    )
}

pub(crate) fn production_sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
            .flatten()
        {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_test_source(&name) {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else if name.ends_with(".rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

pub fn run(root: &Path) -> Result<(), String> {
    let mut reports = Vec::new();
    for source_root in checked_source_roots(root) {
        for file in production_sources(&source_root) {
            let source = fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
            for (line, found) in violations(&source) {
                reports.push(format!("{}:{line}: {found}", relative(root, &file)));
            }
        }
    }
    if reports.is_empty() {
        println!("Lock check passed.");
        return Ok(());
    }
    Err(format!(
        "std locks in production code:\n\n{}\n\n\
         A std lock is poisoned by a panic and fails every later caller; use\n\
         parking_lot::Mutex / RwLock / Condvar (AGENTS.md, \"Locks\").\n",
        reports.join("\n")
    ))
}

/// Every use of a banned std lock in `source`, as (line, what was found).
pub(crate) fn violations(source: &str) -> Vec<(usize, String)> {
    let file = syn::parse_file(source).unwrap_or_else(|error| panic!("parse source: {error}"));
    let mut aliases = SyncAliases::default();
    aliases.visit_file(&file);
    let mut finder = LockFinder {
        sync_aliases: aliases.names,
        found: Vec::new(),
    };
    finder.visit_file(&file);
    finder.found.sort();
    finder.found.dedup();
    finder.found
}

/// The names `std::sync` itself is imported under (`use std::sync;`,
/// `use std::{sync, ...};`, `use std::sync as s;`), so `sync::Mutex` is seen.
#[derive(Default)]
struct SyncAliases {
    names: Vec<String>,
}

impl<'ast> Visit<'ast> for SyncAliases {
    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        for (path, leaf) in flatten(&node.tree) {
            if let Leaf::Name { name, alias } = leaf {
                if is_std(&path) && path.len() == 1 && name == "sync" {
                    self.names.push(alias);
                }
            }
        }
    }
}

struct LockFinder {
    sync_aliases: Vec<String>,
    found: Vec<(usize, String)>,
}

impl LockFinder {
    fn record(&mut self, line: usize, text: String) {
        self.found.push((line, text));
    }

    /// Whether `segments` name a banned lock: `std::sync::Mutex...`, or
    /// `<alias>::Mutex...` for an alias of `std::sync`.
    fn names_banned_lock(&self, segments: &[String]) -> bool {
        let rest = match segments {
            [first, second, rest @ ..] if first == "std" && second == "sync" => rest,
            [alias, rest @ ..] if self.sync_aliases.contains(alias) => rest,
            _ => return false,
        };
        rest.first().is_some_and(|name| is_banned(name))
    }
}

impl<'ast> Visit<'ast> for LockFinder {
    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        let line = node.span().start().line;
        for (path, leaf) in flatten(&node.tree) {
            let banned = match &leaf {
                Leaf::Name { name, .. } => {
                    let mut full = path.clone();
                    full.push(name.clone());
                    self.names_banned_lock(&full)
                }
                // `use std::sync::*` imports both.
                Leaf::Glob => {
                    (is_std(&path) && path.get(1).is_some_and(|segment| segment == "sync"))
                        || (path.len() == 1 && self.sync_aliases.contains(&path[0]))
                }
            };
            if banned {
                self.record(line, format!("use of {}", render(&path, &leaf)));
            }
        }
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        let segments = node
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if self.names_banned_lock(&segments) {
            self.record(node.span().start().line, segments.join("::"));
        }
        syn::visit::visit_path(self, node);
    }

    /// Macro bodies are tokens, not paths: `std::sync::Mutex` written inside
    /// `vec![...]` or `format!(...)` is found by its tokens.
    fn visit_macro(&mut self, node: &'ast Macro) {
        let mut found = Vec::new();
        scan_tokens(node.tokens.clone(), &mut Vec::new(), &mut found);
        for (line, text) in found {
            if self.names_banned_lock(&text) {
                self.record(line, text.join("::"));
            }
        }
        syn::visit::visit_macro(self, node);
    }
}

/// Collects every `a::b::c` identifier run in `tokens`, with its line.
fn scan_tokens(
    tokens: TokenStream,
    run: &mut Vec<(usize, String)>,
    found: &mut Vec<(usize, Vec<String>)>,
) {
    let mut colons = 0;
    for token in tokens {
        match token {
            TokenTree::Ident(ident) => {
                if colons != 2 {
                    flush(run, found);
                }
                run.push((ident.span().start().line, ident.to_string()));
                colons = 0;
            }
            TokenTree::Punct(punct) if punct.as_char() == ':' => colons += 1,
            TokenTree::Group(group) => {
                flush(run, found);
                colons = 0;
                scan_tokens(group.stream(), &mut Vec::new(), found);
            }
            _ => {
                flush(run, found);
                colons = 0;
            }
        }
    }
    flush(run, found);
}

fn flush(run: &mut Vec<(usize, String)>, found: &mut Vec<(usize, Vec<String>)>) {
    if let Some((line, _)) = run.first() {
        found.push((*line, run.iter().map(|(_, name)| name.clone()).collect()));
    }
    run.clear();
}

enum Leaf {
    Name { name: String, alias: String },
    Glob,
}

/// Every leaf of a use tree with the path leading to it, `::std` read as
/// `std`.
fn flatten(tree: &UseTree) -> Vec<(Vec<String>, Leaf)> {
    fn walk(tree: &UseTree, prefix: &mut Vec<String>, leaves: &mut Vec<(Vec<String>, Leaf)>) {
        match tree {
            UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                walk(&path.tree, prefix, leaves);
                prefix.pop();
            }
            UseTree::Name(name) => {
                let name = name.ident.to_string();
                leaves.push(leaf(prefix, name.clone(), name));
            }
            UseTree::Rename(rename) => {
                leaves.push(leaf(
                    prefix,
                    rename.ident.to_string(),
                    rename.rename.to_string(),
                ));
            }
            UseTree::Glob(_) => leaves.push((prefix.clone(), Leaf::Glob)),
            UseTree::Group(group) => {
                for item in &group.items {
                    walk(item, prefix, leaves);
                }
            }
        }
    }
    fn leaf(prefix: &[String], name: String, alias: String) -> (Vec<String>, Leaf) {
        // `use std::sync::{self}` names the module itself.
        if name == "self" {
            let mut path = prefix.to_vec();
            let name = path.pop().unwrap_or_default();
            let alias = if alias == "self" { name.clone() } else { alias };
            return (path, Leaf::Name { name, alias });
        }
        (prefix.to_vec(), Leaf::Name { name, alias })
    }
    let mut leaves = Vec::new();
    walk(tree, &mut Vec::new(), &mut leaves);
    leaves
}

fn is_std(path: &[String]) -> bool {
    path.first().is_some_and(|first| first == "std")
}

fn is_banned(name: &str) -> bool {
    BANNED.iter().any(|banned| name.starts_with(banned))
}

fn render(path: &[String], leaf: &Leaf) -> String {
    let mut text = path.join("::");
    match leaf {
        Leaf::Name { name, .. } => {
            text.push_str("::");
            text.push_str(name);
        }
        Leaf::Glob => text.push_str("::*"),
    }
    text
}

#[cfg(test)]
mod tests;
