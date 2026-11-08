//! `cargo xtask tag`: creates an annotated tag on `HEAD`, and optionally
//! pushes it. A `v<version>` tag starts the release workflow, so it must name
//! the current workspace version.

use std::path::Path;

use crate::git::git;
use crate::version::{workspace_version, Version};

#[derive(Debug, Default)]
pub struct Options {
    pub version: Option<String>,
    pub name: Option<String>,
    pub message: Option<String>,
    pub push: bool,
    pub remote: Option<String>,
    pub dry_run: bool,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value =
                |flag: &str| args.next().cloned().ok_or(format!("{flag} needs a value"));
            match arg.as_str() {
                "--version" => options.version = Some(value("--version")?),
                "--name" => options.name = Some(value("--name")?),
                "--message" | "-m" => options.message = Some(value("--message")?),
                "--remote" => options.remote = Some(value("--remote")?),
                "--push" => options.push = true,
                "--dry-run" => options.dry_run = true,
                other => return Err(format!("unexpected argument `{other}`")),
            }
        }
        if options.version.is_some() && options.name.is_some() {
            return Err("give either --version or --name, not both".into());
        }
        Ok(options)
    }
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
