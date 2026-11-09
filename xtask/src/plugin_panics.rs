//! `cargo xtask check-plugin-panics`: a plugin's production code does not
//! panic on failure (site/content/docs/plugin-rules.en.mdx, "Panics in plugin
//! code"). The compression crate follows the same rule: the kernel calls it
//! with its panics contained, and a panic there costs the session its
//! projection for the request.

use std::fs;
use std::path::Path;

use crate::source_layout::relative;

/// Every `.unwrap()` and `.expect(` in the production sources of the plugin
/// crates (`crates/plugin-*`), of the facade's own plugins
/// (`crates/agent/src/plugins`) and of the compression crate
/// (`crates/compression`).
pub fn run(root: &Path) -> Result<(), String> {
    let mut violations = Vec::new();
    let crates = root.join("crates");
    let mut plugins: Vec<_> = fs::read_dir(&crates)
        .map_err(|error| format!("read {}: {error}", crates.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("plugin-"))
        })
        .collect();
    plugins.sort();
    for plugin in plugins {
        check_plugin_sources_do_not_panic(root, &plugin.join("src"), &mut violations);
    }
    check_plugin_sources_do_not_panic(
        root,
        &crates.join("agent").join("src").join("plugins"),
        &mut violations,
    );
    check_plugin_sources_do_not_panic(
        root,
        &crates.join("compression").join("src"),
        &mut violations,
    );
    if violations.is_empty() {
        println!("Plugin panic check passed.");
        return Ok(());
    }
    Err(format!(
        "plugin sources that panic on failure:\n\n{}\n",
        violations.join("\n")
    ))
}

/// A panic in a plugin is contained (kardia and the kernel's guards catch
/// it), but it is still a bug that fails the plugin: a plugin reports
/// failures as errors instead.
fn check_plugin_sources_do_not_panic(root: &Path, source: &Path, violations: &mut Vec<String>) {
    let mut pending = vec![source.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        {
            let path = entry.expect("directory entry").path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if matches!(
                name,
                "tests" | "tests.rs" | "test_support" | "test_support.rs"
            ) {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else if name.ends_with(".rs") {
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
                for (line, content) in text.lines().enumerate() {
                    if content.contains(".unwrap()") || content.contains(".expect(") {
                        violations.push(format!(
                            "{}:{} panics on failure; Agent plugins return errors",
                            relative(root, &path),
                            line + 1
                        ));
                    }
                }
            }
        }
    }
}
