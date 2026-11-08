//! `cargo xtask check-source-layout`: keeps the Rust module tree shaped the
//! way AGENTS.md describes it, in every package (`xtask` and `crates/*`).
//!
//! - `lib.rs` only declares and re-exports modules; implementation lives in
//!   files of its own. A `mod.rs` may hold its module's code.
//! - `src/` has no `#[path]` attribute and no glob re-export.
//! - Production files hold no inline tests.
//! - A module with production children is `foo/mod.rs`; a module whose only
//!   children are tests is `foo.rs`; a multi-file test directory under `src/`
//!   is rooted at `tests/mod.rs`.
//! - An integration suite is `tests/<suite>.rs` or `tests/<suite>/main.rs`;
//!   shared helpers live in `tests/support/mod.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::{Attribute, Item};

pub fn run(root: &Path) -> Result<(), String> {
    let violations = violations(root)?;
    if violations.is_empty() {
        println!("Rust source layout check passed.");
        return Ok(());
    }
    Err(format!(
        "Rust source layout violations:\n\n{}",
        violations.join("\n\n")
    ))
}

pub fn violations(root: &Path) -> Result<Vec<String>, String> {
    let mut violations = Vec::new();
    for package in package_roots(root) {
        let src = package.join("src");
        check_directory(root, &src, &src, &mut violations)?;
        check_integration_tests(root, &package, &mut violations);
    }
    Ok(violations)
}

/// An item an entry file must not hold.
#[derive(Debug, PartialEq, Eq)]
pub struct EntryItem {
    pub line: usize,
    pub kind: &'static str,
    pub name: String,
}

/// The items of one entry file's source that are not declarations,
/// re-exports, docs or attributes.
pub fn entry_items(source: &str) -> syn::Result<Vec<EntryItem>> {
    let file = syn::parse_file(source)?;
    let mut found = Vec::new();
    collect_entry_items(&file.items, &mut found);
    Ok(found)
}

fn collect_entry_items(items: &[Item], found: &mut Vec<EntryItem>) {
    for item in items {
        let (kind, name, line) = match item {
            Item::Use(_) | Item::ExternCrate(_) => continue,
            Item::Mod(module) => {
                // An inline module follows the same rule: it may only re-export.
                if let Some((_, items)) = &module.content {
                    collect_entry_items(items, found);
                }
                continue;
            }
            // Rust requires proc-macro entry points at the crate root.
            Item::Fn(function) if is_proc_macro(&function.attrs) => continue,
            Item::Fn(function) => named("fn", &function.sig.ident),
            Item::Struct(item) => named("struct", &item.ident),
            Item::Enum(item) => named("enum", &item.ident),
            Item::Union(item) => named("union", &item.ident),
            Item::Trait(item) => named("trait", &item.ident),
            Item::TraitAlias(item) => named("trait alias", &item.ident),
            Item::Type(item) => named("type", &item.ident),
            Item::Const(item) => named("const", &item.ident),
            Item::Static(item) => named("static", &item.ident),
            Item::Impl(item) => ("impl", impl_name(item), line_of(item.impl_token.span)),
            Item::Macro(item) => match &item.ident {
                Some(ident) => named("macro_rules!", ident),
                None => (
                    "macro call",
                    path_name(&item.mac.path),
                    line_of(item.mac.path.span()),
                ),
            },
            _ => ("item", String::new(), line_of(item.span())),
        };
        found.push(EntryItem { line, kind, name });
    }
}

/// A named item, at the line of its name rather than of its attributes.
fn named(kind: &'static str, ident: &syn::Ident) -> (&'static str, String, usize) {
    (kind, ident.to_string(), line_of(ident.span()))
}

fn line_of(span: proc_macro2::Span) -> usize {
    span.start().line
}

fn is_proc_macro(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        path.is_ident("proc_macro")
            || path.is_ident("proc_macro_derive")
            || path.is_ident("proc_macro_attribute")
    })
}

fn impl_name(item: &syn::ItemImpl) -> String {
    let target = match item.self_ty.as_ref() {
        syn::Type::Path(path) => path_name(&path.path),
        _ => "_".into(),
    };
    match &item.trait_ {
        Some((path, _)) => format!("{} for {target}", path_name(path)),
        None => target,
    }
}

fn path_name(path: &syn::Path) -> String {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default()
}

/// `xtask` and every package under `crates/`. A missing or empty `crates/`
/// is a workspace with no crates yet.
pub(crate) fn package_roots(root: &Path) -> Vec<PathBuf> {
    let mut crates: Vec<PathBuf> = sorted_entries(&root.join("crates"))
        .into_iter()
        .filter(|path| path.join("Cargo.toml").is_file())
        .collect();
    crates.insert(0, root.join("xtask"));
    crates
}

pub(crate) fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sorted_entries(directory: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(directory)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    entries.sort();
    entries
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

/// Lines that are comments or doc comments. The rules are about code, and the
/// documentation explaining them naturally names what it forbids.
fn is_prose(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("#[doc")
}

/// Whether `file` is test code: a `tests.rs` file or anything under a `tests`
/// directory below `src`.
fn is_test_file(src: &Path, file: &Path) -> bool {
    file_name(file) == "tests.rs"
        || file
            .strip_prefix(src)
            .unwrap_or(file)
            .components()
            .any(|component| component.as_os_str() == "tests")
}

/// `pub use path::*;` at any visibility. Private glob imports such as
/// `use super::*;` are not re-exports and stay allowed.
fn is_glob_reexport(line: &str) -> bool {
    line.starts_with("pub") && line.contains(" use ") && line.trim_end().ends_with("::*;")
}

/// A module with production children is `foo/mod.rs`; a module whose only
/// children are tests is `foo.rs`; a multi-file test directory is rooted at
/// `tests/mod.rs`.
fn check_directory(
    root: &Path,
    src: &Path,
    directory: &Path,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    for entry in sorted_entries(directory) {
        if entry.is_dir() {
            let name = file_name(&entry);
            if name == "tests" {
                if directory.join("tests.rs").is_file() {
                    violations.push(format!(
                        "{}: test directory with a sibling tests.rs. Root it at tests/mod.rs.",
                        relative(root, &entry)
                    ));
                }
                continue;
            }
            let has_production_children = sorted_entries(&entry)
                .iter()
                .any(|child| !matches!(file_name(child), "mod.rs" | "tests.rs" | "tests"));
            let sibling = directory.join(format!("{name}.rs"));
            if has_production_children && sibling.is_file() {
                violations.push(format!(
                    "{}: module with production children. Move it to {name}/mod.rs.",
                    relative(root, &sibling)
                ));
            }
            if !has_production_children && entry.join("mod.rs").is_file() {
                violations.push(format!(
                    "{}: module whose only children are tests. Move it to {name}.rs.",
                    relative(root, &entry.join("mod.rs"))
                ));
            }
            check_directory(root, src, &entry, violations)?;
        } else if file_name(&entry).ends_with(".rs") {
            check_file(root, src, &entry, violations)?;
        }
    }
    Ok(())
}

fn check_file(
    root: &Path,
    src: &Path,
    file: &Path,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    let contents =
        fs::read_to_string(file).map_err(|error| format!("{}: {error}", relative(root, file)))?;
    let test_file = is_test_file(src, file);
    if file == src.join("lib.rs") {
        let items =
            entry_items(&contents).map_err(|error| format!("{}: {error}", relative(root, file)))?;
        for item in items {
            violations.push(format!(
                "{}:{}: {} `{}` in lib.rs. lib.rs holds only docs, `mod` declarations\n\
                 and re-exports; move it into a named module and re-export it.",
                relative(root, file),
                item.line,
                item.kind,
                item.name
            ));
        }
    }
    for (index, line) in contents.lines().enumerate() {
        if is_prose(line) {
            continue;
        }
        let number = index + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[path") {
            violations.push(format!(
                "{}:{number}: `#[path]` in src/. The module tree is the directory tree;\n\
                 move the file to where `mod` resolves it.",
                relative(root, file)
            ));
        }
        if !test_file && is_glob_reexport(trimmed) {
            violations.push(format!(
                "{}:{number}: glob re-export. Name what the module exposes:\n\
                 `pub use module::{{Item, Other}};`.",
                relative(root, file)
            ));
        }
        if !test_file
            && (trimmed.starts_with("#[test]")
                || trimmed.starts_with("#[tokio::test")
                || trimmed.starts_with("mod tests {"))
        {
            violations.push(format!(
                "{}:{number}: inline test in a production file. Declare\n\
                 `#[cfg(test)] mod tests;` and put the tests in the module's tests.rs.",
                relative(root, file)
            ));
        }
    }
    Ok(())
}

/// A single-file suite is `tests/<suite>.rs`; a multi-file suite is
/// `tests/<suite>/main.rs`, never `tests/<suite>.rs` beside `tests/<suite>/`.
fn check_integration_tests(root: &Path, package: &Path, violations: &mut Vec<String>) {
    let tests = package.join("tests");
    for entry in sorted_entries(&tests) {
        if !entry.is_dir() {
            continue;
        }
        let name = file_name(&entry);
        let suite_root = tests.join(format!("{name}.rs"));
        if suite_root.is_file() {
            violations.push(format!(
                "{}: multi-file suite rooted beside its directory. Move it to {name}/main.rs.",
                relative(root, &suite_root)
            ));
        }
        if name != "support" && !entry.join("main.rs").is_file() {
            violations.push(format!(
                "{}: test directory with no main.rs. A multi-file suite is rooted at\n\
                 tests/<suite>/main.rs; shared helpers live in tests/support/mod.rs.",
                relative(root, &entry)
            ));
        }
    }
}

#[cfg(test)]
mod tests;
