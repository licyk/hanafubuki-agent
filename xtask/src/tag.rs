//! `cargo xtask tag`: creates an annotated tag on `HEAD`, and optionally
//! pushes it. A `v<version>` tag starts the release workflow, so it must name
//! the current workspace version.

use std::path::Path;

use crate::git::git;
use crate::version::{workspace_version, Version};

#[derive(Debug, Default, clap::Args)]
pub struct Options {
    /// Explicit version; must match the workspace version.
    #[arg(long, conflicts_with = "name", value_name = "X.Y.Z")]
    pub version: Option<String>,
    /// Custom tag name instead of a version tag.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// Annotation text (default: the release or tag name).
    #[arg(long, short = 'm', value_name = "TEXT")]
    pub message: Option<String>,
    /// Push the created tag.
    #[arg(long)]
    pub push: bool,
    /// Remote to push to (default: origin).
    #[arg(long, value_name = "NAME")]
    pub remote: Option<String>,
    /// Print the planned tag without creating or pushing it.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(root: &Path, options: &Options) -> Result<(), String> {
    let (tag, message) = tag_for(current_version(root)?, options)?;
    let remote = options.remote.as_deref().unwrap_or("origin");

    if !git(root, &["status", "--porcelain"])?.trim().is_empty() {
        return Err(
            "the working tree has uncommitted changes: commit them first, so the tag names them"
                .into(),
        );
    }
    if git(root, &["tag", "--list", &tag])?.trim() == tag {
        return Err(format!("tag {tag} already exists"));
    }
    if options.push
        && !git(
            root,
            &["ls-remote", "--tags", remote, &format!("refs/tags/{tag}")],
        )?
        .trim()
        .is_empty()
    {
        return Err(format!("tag {tag} already exists on {remote}"));
    }

    let head = git(root, &["rev-parse", "--short", "HEAD"])?;
    let pushing = if options.push {
        format!(", then push it to {remote}")
    } else {
        String::new()
    };
    if options.dry_run {
        println!(
            "Would tag {} as {tag} (\"{message}\"){pushing}",
            head.trim()
        );
        return Ok(());
    }
    git(root, &["tag", "--annotate", &tag, "--message", &message])?;
    println!("Tagged {} as {tag}", head.trim());
    if options.push {
        git(root, &["push", remote, &format!("refs/tags/{tag}")])?;
        println!("Pushed {tag} to {remote}");
    }
    Ok(())
}

/// The tag and its message. A version tag must be the workspace version.
pub fn tag_for(current: Version, options: &Options) -> Result<(String, String), String> {
    let (tag, version) = match (&options.name, &options.version) {
        (Some(name), _) => (name.clone(), None),
        (None, Some(text)) => {
            let version = Version::parse(text.trim_start_matches('v'))?;
            if version != current {
                return Err(format!(
                    "v{version} is not the workspace version {current}: bump it with `cargo xtask version` and commit first"
                ));
            }
            (format!("v{version}"), Some(version))
        }
        (None, None) => (format!("v{current}"), Some(current)),
    };
    if tag.is_empty() || tag.contains(char::is_whitespace) {
        return Err(format!("`{tag}` is not a valid tag name"));
    }
    let message = match (&options.message, version) {
        (Some(message), _) => message.clone(),
        (None, Some(version)) => format!("hanafubuki-agent {version}"),
        (None, None) => tag.clone(),
    };
    Ok((tag, message))
}

fn current_version(root: &Path) -> Result<Version, String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|error| format!("Cargo.toml: {error}"))?;
    workspace_version(&manifest)
}

#[cfg(test)]
mod tests;
