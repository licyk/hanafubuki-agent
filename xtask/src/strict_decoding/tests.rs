use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn found(source: &str) -> Vec<(usize, Option<String>)> {
    occurrences(source)
        .unwrap()
        .into_iter()
        .map(|occurrence| (occurrence.line, occurrence.type_name))
        .collect()
}

fn named(line: usize, name: &str) -> (usize, Option<String>) {
    (line, Some(name.to_string()))
}

#[test]
fn every_spelling_and_position_on_a_type_is_found() {
    let source = r#"
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Alone;

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Last { a: u8 }

#[serde(
    tag = "type",
    deny_unknown_fields,
    rename_all = "snake_case"
)]
enum Tagged { One {} }

#[cfg_attr(feature = "wire", serde(deny_unknown_fields))]
struct Conditional;

struct OnAField {
    #[serde(deny_unknown_fields)]
    inner: u8,
}

enum OnAVariant {
    #[serde(deny_unknown_fields)]
    Variant { a: u8 },
}

mod wire {
    mod nested {
        #[serde(deny_unknown_fields)]
        pub struct Inner;
    }
}
"#;
    assert_eq!(
        found(source),
        vec![
            named(3, "Alone"),
            named(6, "Last"),
            named(9, "Tagged"),
            named(16, "Conditional"),
            named(20, "OnAField"),
            named(25, "OnAVariant"),
            named(31, "wire::nested::Inner"),
        ]
    );
}

#[test]
fn other_attributes_and_prose_are_not_occurrences() {
    let source = r#"
/// Never uses `deny_unknown_fields`.
#[derive(Deserialize)]
#[serde(rename = "deny_unknown_fields", default)]
struct Tolerant {
    #[serde(default)]
    field: Option<u8>,
}

#[doc = "deny_unknown_fields"]
#[cfg_attr(test, derive(Debug))]
struct Documented;

fn body() {
    let deny_unknown_fields = 1;
}
"#;
    assert!(found(source).is_empty());
}

#[test]
fn an_occurrence_outside_a_type_has_no_name() {
    let source = r#"
#[serde(deny_unknown_fields)]
fn not_a_type() {}

macro_rules! define {
    () => {
        #[serde(deny_unknown_fields)]
        struct Generated;
    };
}

fn local() {
    #[serde(deny_unknown_fields)]
    struct Local;
}
"#;
    assert_eq!(
        found(source),
        vec![(2, None), (7, None), named(13, "Local")]
    );
}

/// A throwaway source tree under the system temporary directory.
struct Fixture {
    base: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "xtask-strict-decoding-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        Self { base }
    }

    fn write(&self, path: &str, source: &str) -> &Self {
        let file = self.base.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, source).unwrap();
        self
    }

    fn violations(&self, policy: &str) -> Vec<String> {
        let roots = [self.base.join("crate/src")];
        evaluate(&self.base, &roots, &parse_policy(policy).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

const REQUEST: &str = "#[serde(deny_unknown_fields)]\npub struct Request;\n";

fn entry(path: &str, type_name: &str, category: &str, reason: &str) -> String {
    format!(
        "[[types]]\npath = \"{path}\"\ntype = \"{type_name}\"\n\
         category = \"{category}\"\nreason = \"{reason}\"\n"
    )
}

fn policy(entries: &[String]) -> String {
    format!("version = 1\n{}", entries.join("\n"))
}

#[test]
fn a_listed_type_passes_and_test_sources_are_not_scanned() {
    let fixture = Fixture::new();
    fixture
        .write("crate/src/request.rs", REQUEST)
        .write("crate/src/request/tests.rs", REQUEST)
        .write("crate/src/tests/mod.rs", REQUEST)
        .write("crate/src/test_support.rs", REQUEST)
        .write("crate/src/test_support/mod.rs", REQUEST)
        .write("crate/tests/contract.rs", REQUEST);
    let listed = policy(&[entry(
        "crate/src/request.rs",
        "Request",
        "ipc-request",
        "The argument of a command.",
    )]);
    assert_eq!(fixture.violations(&listed), Vec::<String>::new());
}

#[test]
fn an_unlisted_type_fails() {
    let fixture = Fixture::new();
    fixture.write("crate/src/persisted.rs", REQUEST).write(
        "crate/src/generated.rs",
        "macro_rules! m { () => { #[serde(deny_unknown_fields)] struct S; } }\n",
    );
    let violations = fixture.violations(&policy(&[]));
    assert_eq!(violations.len(), 2, "{violations:#?}");
    assert!(violations[0].starts_with("crate/src/generated.rs:1:"));
    assert!(violations[0].contains("outside a type definition"));
    assert!(violations[1].starts_with("crate/src/persisted.rs:1: `Request`"));
    assert!(violations[1].contains("is not listed"));
}

#[test]
fn a_stale_entry_fails() {
    let fixture = Fixture::new();
    fixture
        .write("crate/src/request.rs", REQUEST)
        .write("crate/src/tolerant.rs", "pub struct Tolerant;\n");
    let violations = fixture.violations(&policy(&[
        entry("crate/src/request.rs", "Request", "ipc-request", "Kept."),
        entry("crate/src/tolerant.rs", "Tolerant", "other", "Lost it."),
        entry(
            "crate/src/gone.rs",
            "Gone",
            "integrity-manifest",
            "Removed.",
        ),
        entry("crate/src/request.rs", "Renamed", "ipc-request", "Renamed."),
    ]));
    assert_eq!(violations.len(), 3, "{violations:#?}");
    assert!(violations
        .iter()
        .all(|violation| violation.contains("remove its entry")));
    assert!(violations[0].starts_with("crate/src/tolerant.rs `Tolerant` (other)"));
    assert!(violations[1].starts_with("crate/src/gone.rs `Gone` (integrity-manifest)"));
    assert!(violations[2].starts_with("crate/src/request.rs `Renamed`"));
}

#[test]
fn duplicate_and_unexplained_entries_fail() {
    let fixture = Fixture::new();
    fixture.write("crate/src/request.rs", REQUEST);
    let violations = fixture.violations(&policy(&[
        entry("crate/src/request.rs", "Request", "ipc-request", "Once."),
        entry("crate/src/request.rs", "Request", "ipc-request", " "),
    ]));
    assert_eq!(violations.len(), 2, "{violations:#?}");
    assert!(violations[0].contains("listed twice"));
    assert!(violations[1].contains("has no reason"));
}

#[test]
fn the_policy_format_is_strict() {
    assert!(parse_policy("version = 2\n")
        .unwrap_err()
        .contains("version 2"));
    let unknown_category = policy(&[entry("a.rs", "A", "persisted", "No.")]);
    assert!(parse_policy(&unknown_category).is_err());
    let unknown_field = format!(
        "{}extra = 1\n",
        policy(&[entry("a.rs", "A", "other", "No.")])
    );
    assert!(parse_policy(&unknown_field).is_err());
}

#[test]
fn the_checked_in_policy_parses() {
    let policy = parse_policy(POLICY).unwrap();
    assert!(!policy.types.is_empty());
}
