// Derived from OpenAI Codex (https://github.com/openai/codex), the tests
// of `codex-rs/core-plugins/src/marketplace_add/source.rs` at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a `file://` URL is a git source.

use super::*;

fn git(url: &str, ref_name: Option<&str>) -> AddSource {
    AddSource::Git {
        url: url.to_string(),
        ref_name: ref_name.map(str::to_string),
    }
}

#[test]
fn github_shorthand_parses_ref_suffix() {
    assert_eq!(
        parse_add_source("owner/repo@main", None).unwrap(),
        git("https://github.com/owner/repo.git", Some("main"))
    );
}

#[test]
fn git_url_parses_fragment_ref() {
    assert_eq!(
        parse_add_source("https://example.com/team/repo.git#v1", None).unwrap(),
        git("https://example.com/team/repo.git", Some("v1"))
    );
}

#[test]
fn explicit_ref_overrides_source_ref() {
    assert_eq!(
        parse_add_source("owner/repo@main", Some("release".to_string())).unwrap(),
        git("https://github.com/owner/repo.git", Some("release"))
    );
}

#[test]
fn github_shorthand_and_git_url_normalize_to_same_source() {
    let expected = git("https://github.com/owner/repo.git", None);
    assert_eq!(parse_add_source("owner/repo", None).unwrap(), expected);
    assert_eq!(
        parse_add_source("https://github.com/owner/repo.git", None).unwrap(),
        expected
    );
    assert_eq!(
        parse_add_source("https://github.com/owner/repo/", None).unwrap(),
        expected
    );
}

#[test]
fn non_github_https_and_ssh_sources_parse_as_git_urls() {
    assert_eq!(
        parse_add_source("https://gitlab.com/owner/repo", None).unwrap(),
        git("https://gitlab.com/owner/repo", None)
    );
    assert_eq!(
        parse_add_source("ssh://git@github.com/owner/repo.git#main", None).unwrap(),
        git("ssh://git@github.com/owner/repo.git", Some("main"))
    );
    assert_eq!(
        parse_add_source("file:///tmp/marketplace.git", None).unwrap(),
        git("file:///tmp/marketplace.git", None)
    );
}

#[test]
fn local_path_source_resolves() {
    let AddSource::Local { path } = parse_add_source(".", None).unwrap() else {
        panic!("expected a local source");
    };
    assert!(path.native().is_absolute());
}

#[test]
fn windows_absolute_paths_look_like_local_paths_on_every_host() {
    assert!(looks_like_local_path(r"C:\Users\alice\marketplace"));
    assert!(looks_like_local_path("C:/Users/alice/marketplace"));
    assert!(looks_like_local_path(r"\\server\share\marketplace"));
    assert!(!looks_like_local_path(r"C:relative\path"));
}

#[test]
fn local_file_and_bad_sources_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("marketplace.json");
    std::fs::write(&file, "{}").unwrap();

    assert_eq!(
        parse_add_source(file.to_str().unwrap(), None)
            .unwrap_err()
            .to_string(),
        "local marketplace source must be a directory, not a file"
    );
    assert_eq!(
        parse_add_source("./marketplace", Some("main".to_string()))
            .unwrap_err()
            .to_string(),
        "--ref is only supported for git marketplace sources"
    );
    assert!(parse_add_source("not a source", None)
        .unwrap_err()
        .to_string()
        .contains("invalid marketplace source format"));
    assert!(parse_add_source("  ", None).is_err());
}
