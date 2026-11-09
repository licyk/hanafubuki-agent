use super::*;

const MARKER: &str = "Modified from Upstream v1 (Apache-2.0).";

const POLICY_TEXT: &str = r#"
version = 1

[[derived]]
path = "crates/port/src"
marker = "Modified from Upstream v1 (Apache-2.0)."
notice = "crates/port"

[[mit]]
path = "crates/port/src/terminal.rs"
holder = "Copyright (c) 2016 Someone"

[[original]]
path = "crates/port/src/lib.rs"
reason = "The crate's own entry."
"#;

const NOTICE: &str = "`crates/port` is ported; `crates/port/src/terminal.rs` keeps its MIT header.";

/// A scratch repository with the given files, removed when dropped.
struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let root =
            std::env::temp_dir().join(format!("xtask-notices-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, contents) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        Self(root)
    }

    fn check(&self, notice: &str) -> Vec<String> {
        check(&self.0, &parse(POLICY_TEXT).unwrap(), notice)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn ported(body: &str) -> String {
    format!("// {MARKER}\n\n{body}")
}

fn terminal() -> String {
    format!(
        "// Derived from a terminal.\n//\n// The MIT License (MIT)\n\
         // Copyright (c) 2016 Someone\n//\n// {MARKER}\nuse std::io;\n"
    )
}

#[test]
fn a_complete_port_passes() {
    let fixture = Fixture::new(
        "complete",
        &[
            ("crates/port/src/lib.rs", "//! Docs.\nmod widget;\n"),
            ("crates/port/src/widget.rs", &ported("fn draw() {}\n")),
            (
                "crates/port/src/widget/tests.rs",
                &ported("#[test]\nfn t() {}\n"),
            ),
            ("crates/port/src/terminal.rs", &terminal()),
            ("crates/port/src/widget/snapshots/a.snap", "---\n"),
        ],
    );
    assert_eq!(fixture.check(NOTICE), Vec::<String>::new());
}

#[test]
fn a_file_without_its_marker_is_named() {
    let fixture = Fixture::new(
        "unmarked",
        &[
            ("crates/port/src/lib.rs", "//! Docs.\n"),
            ("crates/port/src/widget.rs", "fn draw() {}\n"),
            (
                "crates/port/src/late.rs",
                &format!("fn f() {{}}\n// {MARKER}\n"),
            ),
            ("crates/port/src/terminal.rs", &terminal()),
        ],
    );
    let violations = fixture.check(NOTICE);
    assert_eq!(violations.len(), 2, "{violations:?}");
    assert!(violations[0].starts_with("crates/port/src/late.rs:"));
    assert!(violations[1].starts_with("crates/port/src/widget.rs:"));
}

#[test]
fn a_lost_mit_header_and_a_notice_that_names_nothing_are_named() {
    let fixture = Fixture::new(
        "mit",
        &[
            ("crates/port/src/lib.rs", "//! Docs.\n"),
            ("crates/port/src/terminal.rs", &ported("use std::io;\n")),
        ],
    );
    let violations = fixture.check("nothing named here");
    assert_eq!(
        violations,
        vec![
            "NOTICE does not name `crates/port` (crates/port/src)".to_string(),
            "NOTICE does not name `crates/port/src/terminal.rs`".to_string(),
            "crates/port/src/terminal.rs: the MIT header is gone".to_string(),
        ]
    );
}

#[test]
fn a_stale_original_entry_is_named() {
    let fixture = Fixture::new(
        "stale",
        &[
            ("crates/port/src/widget.rs", &ported("fn draw() {}\n")),
            ("crates/port/src/terminal.rs", &terminal()),
        ],
    );
    let violations = fixture.check(NOTICE);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("listed as original but no longer exists"));
}

#[test]
fn the_header_is_the_leading_comment_block() {
    assert_eq!(header("// a\n\n//! b\nuse c;\n// d\n"), "// a\n\n//! b");
    assert_eq!(header("use c;\n// a\n"), "");
}

#[test]
fn the_repository_policy_parses() {
    let policy = parse(POLICY).unwrap();
    assert!(!policy.derived.is_empty());
    assert!(!policy.mit.is_empty());
}
