//! `cargo xtask check-config-reference`: every setting the configuration
//! declares has its row in the configuration reference, in both languages.
//!
//! A setting is declared by `SettingSpec::new("key", …)` in the production
//! sources of `crates/config/src`, its key a string literal; the rows are
//! the first cells of the tables under the reference's `## config.toml`
//! heading (`` | `memory.enabled` | … ``). The tables carry prose and links
//! a spec does not, so they are checked rather than generated.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Expr, ExprCall, Lit, Macro, Token};

use crate::source_layout::relative;

const SOURCES: &str = "crates/config/src";
const REFERENCES: [&str; 2] = [
    "site/content/docs/config-reference.en.mdx",
    "site/content/docs/config-reference.mdx",
];
const SECTION: &str = "## config.toml";

pub fn run(root: &Path) -> Result<(), String> {
    let mut violations = Vec::new();
    let mut keys = Vec::new();
    for file in production_files(&root.join(SOURCES)) {
        let source = fs::read_to_string(&file)
            .map_err(|error| format!("read {}: {error}", file.display()))?;
        let name = relative(root, &file);
        let found = spec_keys(&source).map_err(|error| format!("parse {name}: {error}"))?;
        for (line, key) in found {
            match key {
                Some(key) => keys.push((key, format!("{name}:{line}"))),
                None => violations.push(format!(
                    "{name}:{line}: SettingSpec::new takes its key as a string literal"
                )),
            }
        }
    }
    if keys.is_empty() {
        return Err(format!("no SettingSpec::new in {SOURCES}"));
    }
    for reference in REFERENCES {
        let text = fs::read_to_string(root.join(reference))
            .map_err(|error| format!("read {reference}: {error}"))?;
        let rows = section_rows(&text);
        for (key, at) in &keys {
            if !rows.contains(key) {
                violations.push(format!(
                    "{reference}: no row for `{key}` (declared at {at})"
                ));
            }
        }
    }
    if violations.is_empty() {
        println!("Config reference check passed.");
        return Ok(());
    }
    Err(format!(
        "settings missing from the configuration reference:\n\n{}\n",
        violations.join("\n")
    ))
}

/// The key of each `SettingSpec::new` call in `source`, by line; `None` for
/// a key that is not a string literal.
fn spec_keys(source: &str) -> Result<Vec<(usize, Option<String>)>, String> {
    let file = syn::parse_file(source).map_err(|error| error.to_string())?;
    let mut finder = SpecFinder::default();
    finder.visit_file(&file);
    Ok(finder.keys)
}

#[derive(Default)]
struct SpecFinder {
    keys: Vec<(usize, Option<String>)>,
}

impl<'ast> Visit<'ast> for SpecFinder {
    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if is_spec_new(&node.func) {
            let key = match node.args.first() {
                Some(Expr::Lit(literal)) => match &literal.lit {
                    Lit::Str(text) => Some(text.value()),
                    _ => None,
                },
                _ => None,
            };
            self.keys.push((node.span().start().line, key));
        }
        syn::visit::visit_expr_call(self, node);
    }

    /// A macro's body is tokens to syn: one of expressions (`vec![…]`) is
    /// read as them.
    fn visit_macro(&mut self, node: &'ast Macro) {
        if let Ok(expressions) =
            node.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
        {
            for expression in &expressions {
                self.visit_expr(expression);
            }
        }
    }
}

fn is_spec_new(func: &Expr) -> bool {
    let Expr::Path(path) = func else {
        return false;
    };
    let segments: Vec<String> = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    segments.ends_with(&["SettingSpec".to_string(), "new".to_string()])
}

/// The first cell's code of each table row under `## config.toml`, up to
/// the next heading of its level.
fn section_rows(text: &str) -> BTreeSet<String> {
    text.lines()
        .skip_while(|line| !line.starts_with(SECTION))
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|cell| cell.split_once('`'))
        .map(|(key, _)| key.to_string())
        .collect()
}

/// The `.rs` files under `directory`, but tests.
fn production_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if matches!(name, "tests" | "tests.rs") {
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

#[cfg(test)]
mod tests;
