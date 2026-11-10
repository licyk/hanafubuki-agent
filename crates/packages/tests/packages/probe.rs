// Derived from OpenAI Codex (https://github.com/openai/codex), the tests
// of `codex-rs/utils/plugins/src/plugin_namespace.rs` at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the probe returns the format; this
// project's `.hanafubuki-plugin` is probed first.

use std::fs;

use hanafubuki_agent_packages::{find_manifest, ManifestFormat, AGENT_PLUGINS_SCHEMA};

use super::write;

#[test]
fn probes_hanafubuki_then_agent_plugins_then_codex_claude_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("sample");
    let formats = [
        ManifestFormat::Hanafubuki,
        ManifestFormat::AgentPlugins,
        ManifestFormat::Codex,
        ManifestFormat::Claude,
        ManifestFormat::Cursor,
    ];
    for format in formats {
        let contents = if format == ManifestFormat::AgentPlugins {
            format!(r#"{{"$schema":"{AGENT_PLUGINS_SCHEMA}","name":"sample"}}"#)
        } else {
            r#"{"name":"sample"}"#.to_string()
        };
        write(&root.join(format.relative_path()), &contents);
    }

    for format in formats {
        let path = root.join(format.relative_path());
        assert_eq!(find_manifest(&root), Some((format, path.clone())));
        fs::remove_file(path).unwrap();
    }
    assert_eq!(find_manifest(&root), None);
}

#[test]
fn ignores_unrelated_root_plugin_manifest_before_legacy_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("sample");
    write(&root.join("plugin.json"), r#"{"name":"npm-package"}"#);
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    );

    assert_eq!(
        find_manifest(&root),
        Some((
            ManifestFormat::Codex,
            root.join(".codex-plugin/plugin.json")
        ))
    );
}

#[test]
fn an_unsupported_agent_plugins_schema_still_selects_the_root_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("sample");
    write(
        &root.join("plugin.json"),
        r#"{"$schema":"https://agent-plugins.org/schemas/2.0.0/plugin.schema.json","name":"sample"}"#,
    );
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    );

    assert_eq!(
        find_manifest(&root).map(|(format, _)| format),
        Some(ManifestFormat::AgentPlugins)
    );
}

#[test]
fn rejects_nonregular_root_plugin_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("sample");
    fs::create_dir_all(root.join("plugin.json")).unwrap();
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    );

    assert_eq!(find_manifest(&root), None);
}

#[test]
fn rejects_nonregular_legacy_manifest_before_lower_precedence_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("sample");
    fs::create_dir_all(root.join(".codex-plugin/plugin.json")).unwrap();
    write(
        &root.join(".claude-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    );

    assert_eq!(find_manifest(&root), None);
}

#[cfg(unix)]
mod symbolic_links {
    use std::fs;
    use std::os::unix::fs::symlink;

    use hanafubuki_agent_packages::{find_manifest, AGENT_PLUGINS_SCHEMA};

    use crate::write;

    #[test]
    fn rejects_symlinked_root_plugin_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("sample");
        let target = directory.path().join("manifest.json");
        write(
            &target,
            &format!(r#"{{"$schema":"{AGENT_PLUGINS_SCHEMA}","name":"sample"}}"#),
        );
        fs::create_dir_all(&root).unwrap();
        symlink(&target, root.join("plugin.json")).unwrap();
        write(
            &root.join(".codex-plugin/plugin.json"),
            r#"{"name":"sample"}"#,
        );

        assert_eq!(find_manifest(&root), None);
    }

    #[test]
    fn rejects_symlinked_legacy_manifest_before_lower_precedence_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("sample");
        write(&root.join("benign.json"), r#"{"name":"sample"}"#);
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"sample"}"#,
        );
        fs::create_dir_all(root.join(".codex-plugin")).unwrap();
        symlink("../benign.json", root.join(".codex-plugin/plugin.json")).unwrap();

        assert_eq!(find_manifest(&root), None);
    }

    #[test]
    fn rejects_symlinked_legacy_manifest_directory_before_lower_precedence_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("sample");
        let elsewhere = directory.path().join("manifest-directory");
        write(&elsewhere.join("plugin.json"), r#"{"name":"sample"}"#);
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"sample"}"#,
        );
        symlink(&elsewhere, root.join(".codex-plugin")).unwrap();

        assert_eq!(find_manifest(&root), None);
    }

    #[test]
    fn rejects_symlinked_claude_manifest_before_cursor_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("sample");
        write(&root.join("benign.json"), r#"{"name":"sample"}"#);
        write(
            &root.join(".cursor-plugin/plugin.json"),
            r#"{"name":"sample"}"#,
        );
        fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        symlink(
            root.join("benign.json"),
            root.join(".claude-plugin/plugin.json"),
        )
        .unwrap();

        assert_eq!(find_manifest(&root), None);
    }

    #[test]
    fn rejects_a_symlinked_hanafubuki_manifest_before_the_others() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("sample");
        write(&root.join("benign.json"), r#"{"name":"sample"}"#);
        write(
            &root.join(".codex-plugin/plugin.json"),
            r#"{"name":"sample"}"#,
        );
        fs::create_dir_all(root.join(".hanafubuki-plugin")).unwrap();
        symlink(
            "../benign.json",
            root.join(".hanafubuki-plugin/plugin.json"),
        )
        .unwrap();

        assert_eq!(find_manifest(&root), None);
    }
}
