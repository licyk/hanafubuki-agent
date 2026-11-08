//! Flags Rust files and functions far larger than the rest of the codebase.
//!
//! Size alone does not make a module wrong: an exhaustive dispatch, a data
//! table, or a set of contract types can be long and still be one thing. What
//! a size limit catches is growth nobody decided on, and that is hard to see
//! from inside a change. So the limits sit well above the typical module, and
//! every module over them is an exception recorded with its reason and the
//! size it was reviewed at. Growing past that ceiling fails; shrinking passes;
//! an exception that no longer applies has to be removed.
//!
//! Counts are code lines: blank lines and comments, documentation included, do
//! not count, so explaining code never pushes it over a limit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::source_layout::{package_roots, relative};

const POLICY: &str = include_str!("../../architecture/module-size-policy.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModuleSizePolicy {
    version: u32,
    limits: Limits,
    #[serde(default)]
    exceptions: Vec<Exception>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Limits {
    production_file: usize,
    test_file: usize,
    production_function: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exception {
    /// Repository-relative path with `/` separators.
    path: String,
    /// The function's qualified name within the file (`Type::method`,
    /// `module::function`); absent for a whole-file exception.
    function: Option<String>,
    /// The code-line count the module was reviewed at.
    ceiling: usize,
    reason: String,
}

/// One measured source file.
#[derive(Debug)]
struct FileMeasurement {
    path: String,
    test: bool,
    lines: usize,
    /// Code lines per function, keyed by qualified name. Functions sharing a
    /// name (platform `cfg` variants, one per impl) keep the largest.
    functions: BTreeMap<String, usize>,
}

fn policy() -> ModuleSizePolicy {
    let policy = toml::from_str::<ModuleSizePolicy>(POLICY)
        .unwrap_or_else(|error| panic!("parse module-size-policy.toml: {error}"));
    assert_eq!(policy.version, 1, "unsupported module size policy version");
    policy
}

/// Whether each line holds code: not blank and not a comment.
fn code_line_flags(source: &str) -> Vec<bool> {
    let mut in_block_comment = false;
    source
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if in_block_comment {
                if trimmed.contains("*/") {
                    in_block_comment = false;
                }
                return false;
            }
            if trimmed.is_empty() || trimmed.starts_with("//") {
                return false;
            }
            if trimmed.starts_with("/*") {
                in_block_comment = !trimmed.contains("*/");
                return false;
            }
            true
        })
        .collect()
}

/// Code lines in a 1-based inclusive line range.
fn code_lines_between(flags: &[bool], first: usize, last: usize) -> usize {
    flags
        .iter()
        .take(last)
        .skip(first.saturating_sub(1))
        .filter(|code| **code)
        .count()
}

fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default(),
        other => quote::quote!(#other).to_string().replace(' ', ""),
    }
}

/// Record a function's code lines from its `fn` keyword to its closing brace.
fn record_function(
    functions: &mut BTreeMap<String, usize>,
    flags: &[bool],
    name: String,
    sig: &syn::Signature,
    block: &syn::Block,
) {
    let first = sig.fn_token.span.start().line;
    let last = block.brace_token.span.close().end().line;
    let lines = code_lines_between(flags, first, last);
    let entry = functions.entry(name).or_default();
    *entry = (*entry).max(lines);
}

/// Every function with a body reachable as a module, impl, or trait item.
/// Functions nested inside function bodies are part of their parent's count.
fn collect_functions(
    items: &[syn::Item],
    prefix: &str,
    flags: &[bool],
    functions: &mut BTreeMap<String, usize>,
) {
    for item in items {
        match item {
            syn::Item::Fn(function) => record_function(
                functions,
                flags,
                format!("{prefix}{}", function.sig.ident),
                &function.sig,
                &function.block,
            ),
            syn::Item::Impl(implementation) => {
                let owner = type_name(&implementation.self_ty);
                for item in &implementation.items {
                    if let syn::ImplItem::Fn(method) = item {
                        record_function(
                            functions,
                            flags,
                            format!("{prefix}{owner}::{}", method.sig.ident),
                            &method.sig,
                            &method.block,
                        );
                    }
                }
            }
            syn::Item::Trait(definition) => {
                for item in &definition.items {
                    if let syn::TraitItem::Fn(method) = item {
                        if let Some(block) = &method.default {
                            record_function(
                                functions,
                                flags,
                                format!("{prefix}{}::{}", definition.ident, method.sig.ident),
                                &method.sig,
                                block,
                            );
                        }
                    }
                }
            }
            syn::Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    let prefix = format!("{prefix}{}::", module.ident);
                    collect_functions(items, &prefix, flags, functions);
                }
            }
            _ => {}
        }
    }
}

/// Measure one file's source: its code lines and each function's.
fn measure_source(source: &str) -> Result<(usize, BTreeMap<String, usize>), String> {
    let flags = code_line_flags(source);
    let file = syn::parse_file(source).map_err(|error| error.to_string())?;
    let mut functions = BTreeMap::new();
    collect_functions(&file.items, "", &flags, &mut functions);
    Ok((flags.iter().filter(|code| **code).count(), functions))
}

/// Test code: `tests.rs`, anything under a `tests` directory, and the
/// `test_support`/`test_access` helper modules.
fn is_test_path(path: &str) -> bool {
    path.split('/').any(|component| {
        matches!(
            component,
            "tests" | "tests.rs" | "test_support" | "test_support.rs" | "test_access.rs"
        )
    })
}

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn measure_repository(root: &Path) -> Vec<FileMeasurement> {
    let mut files = Vec::new();
    for package in package_roots(root) {
        rust_files(&package.join("src"), &mut files);
        rust_files(&package.join("tests"), &mut files);
    }
    files
        .into_iter()
        .map(|file| {
            let path = relative(root, &file);
            let source =
                fs::read_to_string(&file).unwrap_or_else(|error| panic!("read {path}: {error}"));
            let (lines, functions) =
                measure_source(&source).unwrap_or_else(|error| panic!("parse {path}: {error}"));
            FileMeasurement {
                test: is_test_path(&path),
                path,
                lines,
                functions,
            }
        })
        .collect()
}

fn within(
    subject: &str,
    measured: usize,
    limit: usize,
    kind: &str,
    exception: Option<&Exception>,
    violations: &mut Vec<String>,
) {
    match exception {
        None if measured > limit => violations.push(format!(
            "{subject}: {measured} code lines exceeds the {limit}-line {kind} limit."
        )),
        Some(exception) if measured > exception.ceiling => violations.push(format!(
            "{subject}: {measured} code lines grew past its reviewed ceiling of {}.",
            exception.ceiling
        )),
        Some(_) if measured <= limit => violations.push(format!(
            "{subject}: {measured} code lines is within the {limit}-line {kind} limit; \
             remove its exception from module-size-policy.toml."
        )),
        _ => {}
    }
}

/// Every violation of `policy` by `files`.
fn evaluate(policy: &ModuleSizePolicy, files: &[FileMeasurement]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut file_exceptions = BTreeMap::new();
    let mut function_exceptions = BTreeMap::new();
    for exception in &policy.exceptions {
        let key = (exception.path.as_str(), exception.function.as_deref());
        let duplicate = match exception.function.as_deref() {
            None => file_exceptions
                .insert(exception.path.as_str(), exception)
                .is_some(),
            Some(function) => function_exceptions
                .insert((exception.path.as_str(), function), exception)
                .is_some(),
        };
        if duplicate {
            violations.push(format!("duplicate module size exception: {key:?}"));
        }
        if exception.reason.trim().is_empty() {
            violations.push(format!("module size exception has no reason: {key:?}"));
        }
    }

    let mut seen = BTreeSet::new();
    for file in files {
        let (limit, kind) = if file.test {
            (policy.limits.test_file, "test file")
        } else {
            (policy.limits.production_file, "production file")
        };
        let file_exception = file_exceptions.get(file.path.as_str()).copied();
        if file_exception.is_some() {
            seen.insert((file.path.clone(), None));
        }
        within(
            &file.path,
            file.lines,
            limit,
            kind,
            file_exception,
            &mut violations,
        );
        if file.test {
            continue;
        }
        for (function, lines) in &file.functions {
            let exception = function_exceptions
                .get(&(file.path.as_str(), function.as_str()))
                .copied();
            if exception.is_some() {
                seen.insert((file.path.clone(), Some(function.clone())));
            }
            within(
                &format!("{} `{function}`", file.path),
                *lines,
                policy.limits.production_function,
                "production function",
                exception,
                &mut violations,
            );
        }
    }

    for exception in &policy.exceptions {
        let key = (exception.path.clone(), exception.function.clone());
        if !seen.contains(&key) {
            let subject = match &exception.function {
                Some(function) => format!("{} `{function}`", exception.path),
                None => exception.path.clone(),
            };
            violations.push(format!(
                "{subject}: no longer exists as a measured production module; \
                 remove its exception from module-size-policy.toml."
            ));
        }
    }
    violations
}

pub fn run(root: &Path) -> Result<(), String> {
    let violations = evaluate(&policy(), &measure_repository(root));
    if violations.is_empty() {
        println!("Module size check passed.");
        return Ok(());
    }
    Err(format!(
        "module size violations:\n\n{}\n\n\
         A limit asks for a review, not a split to fit the number. Split along \
         responsibilities (AGENTS.md, \"Module size\"); if the \
         module is cohesive, record an exception with its reason and current \
         size in architecture/module-size-policy.toml and say so in \
         the handoff.\n",
        violations.join("\n")
    ))
}

/// The largest production files, test files, and production functions, each
/// marked with its limit or exception.
pub fn report(root: &Path, top: usize) -> String {
    let policy = policy();
    let files = measure_repository(root);
    let excepted = |path: &str, function: Option<&str>| {
        policy
            .exceptions
            .iter()
            .any(|exception| exception.path == path && exception.function.as_deref() == function)
    };
    let mark = |measured: usize, limit: usize, has_exception: bool| match (
        measured > limit,
        has_exception,
    ) {
        (_, true) => "  [exception]",
        (true, false) => "  [over limit]",
        (false, false) => "",
    };
    let mut output = String::new();
    for (title, test, limit) in [
        ("production files", false, policy.limits.production_file),
        ("test files", true, policy.limits.test_file),
    ] {
        let mut rows: Vec<_> = files.iter().filter(|file| file.test == test).collect();
        rows.sort_by(|left, right| {
            right
                .lines
                .cmp(&left.lines)
                .then(left.path.cmp(&right.path))
        });
        output.push_str(&format!("Largest {title} (limit {limit} code lines):\n"));
        for file in rows.into_iter().take(top) {
            output.push_str(&format!(
                "  {:5}  {}{}\n",
                file.lines,
                file.path,
                mark(file.lines, limit, excepted(&file.path, None))
            ));
        }
        output.push('\n');
    }
    let limit = policy.limits.production_function;
    let mut functions: Vec<_> = files
        .iter()
        .filter(|file| !file.test)
        .flat_map(|file| {
            file.functions
                .iter()
                .map(move |(name, lines)| (*lines, file.path.as_str(), name.as_str()))
        })
        .collect();
    functions.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(right.1)));
    output.push_str(&format!(
        "Largest production functions (limit {limit} code lines):\n"
    ));
    for (lines, path, name) in functions.into_iter().take(top) {
        output.push_str(&format!(
            "  {lines:5}  {path} `{name}`{}\n",
            mark(lines, limit, excepted(path, Some(name)))
        ));
    }
    output
}

#[cfg(test)]
mod tests;
