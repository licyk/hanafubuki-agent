//! `cargo xtask check-dependencies`: the direction of the workspace's
//! dependencies, read from `cargo metadata`.
//!
//! Hanafubuki depends on this repository, never the reverse, so no workspace
//! package may depend on a `hanafubuki*` package that is not one of its own
//! members, from any source. Inside the workspace, each member may depend
//! only on the members its row of [`LAYERS`] allows.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

/// Each member and the members it may depend on (normal, dev or build). The
/// table is enforced once it has rows: every member then needs one.
const LAYERS: &[(&str, &[&str])] = &[];

/// The part of `cargo metadata --no-deps` the check reads. Without the
/// dependency graph, `packages` holds exactly the workspace members.
#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub dependencies: Vec<Dependency>,
}

/// A declared dependency. `name` is the package name, even when the
/// dependency is renamed in the manifest.
#[derive(Debug, Deserialize)]
pub struct Dependency {
    pub name: String,
}

pub fn run(root: &Path) -> Result<(), String> {
    let violations = violations(&metadata(root)?, LAYERS);
    if violations.is_empty() {
        println!("Dependency check passed.");
        return Ok(());
    }
    Err(format!(
        "dependency violations:\n  {}",
        violations.join("\n  ")
    ))
}

/// Every dependency on a foreign `hanafubuki*` package, and, once `layers`
/// has rows, every internal dependency its member's row does not allow.
pub fn violations(metadata: &Metadata, layers: &[(&str, &[&str])]) -> Vec<String> {
    let members: BTreeSet<&str> = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    let mut violations = Vec::new();
    for package in &metadata.packages {
        for dependency in &package.dependencies {
            let name = dependency.name.as_str();
            if name.starts_with("hanafubuki") && !members.contains(name) {
                violations.push(format!(
                    "{} depends on {name}, a Hanafubuki crate: this repository depends on no Hanafubuki crate",
                    package.name
                ));
            }
        }
    }
    if !layers.is_empty() {
        layer_violations(metadata, &members, layers, &mut violations);
    }
    violations
}

fn layer_violations(
    metadata: &Metadata,
    members: &BTreeSet<&str>,
    layers: &[(&str, &[&str])],
    violations: &mut Vec<String>,
) {
    for (name, _) in layers {
        if !members.contains(name) {
            violations.push(format!(
                "the layer table names {name}, which is not a workspace member"
            ));
        }
    }
    for package in &metadata.packages {
        let Some((_, allowed)) = layers.iter().find(|(name, _)| *name == package.name) else {
            violations.push(format!(
                "{} has no row in the layer table: add it with the members it may depend on",
                package.name
            ));
            continue;
        };
        for dependency in &package.dependencies {
            let name = dependency.name.as_str();
            if members.contains(name) && !allowed.contains(&name) {
                violations.push(format!(
                    "{} depends on {name}, which its layer does not allow",
                    package.name
                ));
            }
        }
    }
}

fn metadata(root: &Path) -> Result<Metadata, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(root)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo metadata` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("`cargo metadata` output: {error}"))
}

#[cfg(test)]
mod tests;
