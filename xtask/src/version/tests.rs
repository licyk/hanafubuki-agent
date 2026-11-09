use super::*;

const MANIFEST: &str = r#"[workspace]
members = ["crates/*", "xtask"]

[workspace.package]
version = "0.1.3"  # the release version
edition = "2021"

[workspace.dependencies]
hanafubuki-agent-kernel = { path = "crates/kernel", version = "0.1.3" }
hanafubuki-agent = { path = "crates/agent", version = "0.1.3" }
foundation = { package = "hanafubuki-agent-foundation", path = "crates/foundation", version = "0.1.3" }
serde = { version = "1", features = ["derive"] }
tokio = "1.47"
"#;

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn members() -> BTreeSet<String> {
    [
        "hanafubuki-agent",
        "hanafubuki-agent-foundation",
        "hanafubuki-agent-kernel",
        "xtask",
    ]
    .map(String::from)
    .into()
}

#[test]
fn bumps_each_level() {
    assert_eq!(version("0.1.3").bump(Level::Patch), version("0.1.4"));
    assert_eq!(version("0.1.3").bump(Level::Minor), version("0.2.0"));
    assert_eq!(version("0.4.2").bump(Level::Major), version("1.0.0"));
    assert_eq!(version("1.2.3").bump(Level::Minor), version("1.3.0"));
}

#[test]
fn refuses_what_is_not_major_minor_patch() {
    for text in ["0.1", "0.1.0-alpha", "1.2.3.4", "x.1.0", ""] {
        assert!(Version::parse(text).is_err(), "{text}");
    }
}

#[test]
fn requirement_follows_semver_compatibility() {
    assert_eq!(version("0.1.9").requirement(), "0.1");
    assert_eq!(version("1.4.0").requirement(), "1");
}

#[test]
fn bumps_the_package_and_internal_dependency_versions_only() {
    assert_eq!(workspace_version(MANIFEST).unwrap(), version("0.1.3"));
    let bumped = bump_manifest(MANIFEST, version("0.2.0"), &members()).unwrap();
    assert_eq!(workspace_version(&bumped).unwrap(), version("0.2.0"));
    assert!(bumped
        .contains(r#"hanafubuki-agent-kernel = { path = "crates/kernel", version = "0.2.0" }"#));
    assert!(bumped.contains(r#"hanafubuki-agent = { path = "crates/agent", version = "0.2.0" }"#));
    assert!(bumped.contains(
        r#"foundation = { package = "hanafubuki-agent-foundation", path = "crates/foundation", version = "0.2.0" }"#
    ));
    assert!(bumped.contains(r#"serde = { version = "1", features = ["derive"] }"#));
    assert!(bumped.contains(r#"tokio = "1.47""#));
    assert!(bumped.contains("# the release version"));
}

#[test]
fn a_manifest_without_workspace_dependencies_bumps_its_version() {
    let manifest = "[workspace]\n\n[workspace.package]\nversion = \"0.1.0\"\n";
    assert_eq!(
        bump_manifest(manifest, version("0.1.1"), &members()).unwrap(),
        "[workspace]\n\n[workspace.package]\nversion = \"0.1.1\"\n"
    );
}

#[test]
fn rewrites_member_requirements_and_nothing_else() {
    let text = "hanafubuki-agent = \"0.1\"\nhanafubuki-agent-kernel = \"0.1\"\nserde = \"0.1\"\nmyhanafubuki-agent = \"0.1\"\n";
    assert_eq!(
        bump_snippets(text, "0.1", "0.2", &members()),
        "hanafubuki-agent = \"0.2\"\nhanafubuki-agent-kernel = \"0.2\"\nserde = \"0.1\"\nmyhanafubuki-agent = \"0.1\"\n"
    );
}

#[test]
fn plans_the_files_of_a_workspace() {
    let root = scratch("plan");
    write(&root, "Cargo.toml", MANIFEST);
    write(
        &root,
        "crates/agent/Cargo.toml",
        "[package]\nname = \"hanafubuki-agent\"\n",
    );
    write(
        &root,
        "crates/kernel/Cargo.toml",
        "[package]\nname = \"hanafubuki-agent-kernel\"\n",
    );
    write(&root, "xtask/Cargo.toml", "[package]\nname = \"xtask\"\n");
    write(&root, "README.md", "hanafubuki-agent = \"0.1\"\n");
    write(
        &root,
        "site/content/docs/installation.mdx",
        "hanafubuki-agent = \"0.1\"\n",
    );
    write(
        &root,
        "site/content/docs/embedding/host.en.mdx",
        "hanafubuki-agent-kernel = \"0.1\"\n",
    );
    write(
        &root,
        "site/content/docs/meta.json",
        "hanafubuki-agent = \"0.1\"\n",
    );

    let (old, new, changes) = plan(&root, Level::Patch).unwrap();
    assert_eq!((old, new), (version("0.1.3"), version("0.1.4")));
    let names = |changes: &[Change]| -> Vec<PathBuf> {
        changes
            .iter()
            .map(|c| c.path.strip_prefix(&root).unwrap().to_path_buf())
            .collect()
    };
    // a patch keeps the requirement users write
    assert_eq!(names(&changes), [PathBuf::from("Cargo.toml")]);

    let (_, new, changes) = plan(&root, Level::Minor).unwrap();
    assert_eq!(new, version("0.2.0"));
    assert_eq!(
        names(&changes),
        [
            "Cargo.toml",
            "README.md",
            "site/content/docs/embedding/host.en.mdx",
            "site/content/docs/installation.mdx",
        ]
        .map(PathBuf::from)
    );
    assert_eq!(changes[1].contents, "hanafubuki-agent = \"0.2\"\n");
    // the foundation crate is not a member of this workspace, so its entry stays
    assert!(changes[0]
        .contents
        .contains(r#"path = "crates/kernel", version = "0.2.0""#));
    assert!(changes[0]
        .contents
        .contains(r#"path = "crates/foundation", version = "0.1.3""#));
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xtask-version-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}
