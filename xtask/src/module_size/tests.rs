use super::*;

fn limits() -> Limits {
    Limits {
        production_file: 10,
        test_file: 20,
        production_function: 5,
    }
}

fn policy_with(exceptions: Vec<Exception>) -> ModuleSizePolicy {
    ModuleSizePolicy {
        version: 1,
        limits: limits(),
        exceptions,
    }
}

fn exception(path: &str, function: Option<&str>, ceiling: usize) -> Exception {
    Exception {
        path: path.to_string(),
        function: function.map(str::to_string),
        ceiling,
        reason: "reviewed".to_string(),
    }
}

fn file(path: &str, lines: usize, functions: &[(&str, usize)]) -> FileMeasurement {
    FileMeasurement {
        path: path.to_string(),
        test: is_test_path(path),
        lines,
        functions: functions
            .iter()
            .map(|(name, lines)| (name.to_string(), *lines))
            .collect(),
    }
}

#[test]
fn blank_lines_and_comments_are_not_code() {
    let source = "\
//! Module docs.

/// Item docs.
fn one() {}
/* a block
   comment */
/* one-line block */
    // indented comment
let x = 1; // trailing comment still code
";
    let flags = code_line_flags(source);
    assert_eq!(flags.iter().filter(|code| **code).count(), 2);
    assert!(flags[3] && flags[8]);
}

#[test]
fn functions_are_measured_by_qualified_name_from_fn_to_closing_brace() {
    let source = r##"
/// Docs above the function are not part of it.
#[inline]
fn free() {
    // a comment line inside does not count
    let text = r#"{ unbalanced { braces"#;
    fn nested() {
        let inner = 1;
    }
}

struct Holder;

impl Holder {
    fn method(&self) {
        let a = 1;
    }
}

trait Behaviour {
    fn required(&self);
    fn provided(&self) {
        let b = 2;
    }
}

mod inline {
    fn inside() {}
}

#[cfg(unix)]
fn platform() {
    let unix = 1;
}

#[cfg(windows)]
fn platform() {
    let windows = 1;
    let more = 2;
}
"##;
    let (_, functions) = measure_source(source).unwrap();
    assert_eq!(functions["free"], 6, "{functions:?}");
    assert!(!functions.contains_key("nested"), "{functions:?}");
    assert_eq!(functions["Holder::method"], 3);
    assert_eq!(functions["Behaviour::provided"], 3);
    assert!(!functions.contains_key("Behaviour::required"));
    assert_eq!(functions["inline::inside"], 1);
    assert_eq!(
        functions["platform"], 4,
        "a name shared by cfg variants keeps the largest"
    );
}

#[test]
fn test_paths_are_recognised() {
    assert!(is_test_path("crates/a/src/foo/tests.rs"));
    assert!(is_test_path("crates/a/src/foo/tests/fixtures.rs"));
    assert!(is_test_path("crates/a/tests/suite/main.rs"));
    assert!(is_test_path("crates/a/src/test_support.rs"));
    assert!(is_test_path("crates/a/src/agent/service/test_access.rs"));
    assert!(!is_test_path("crates/a/src/contests.rs"));
    assert!(!is_test_path("crates/a/src/service.rs"));
}

#[test]
fn modules_within_their_limits_pass() {
    let files = [
        file("crates/a/src/lib.rs", 10, &[("run", 5)]),
        file("crates/a/src/tests.rs", 20, &[("long_test", 50)]),
    ];
    assert!(evaluate(&policy_with(Vec::new()), &files).is_empty());
}

#[test]
fn a_module_over_its_limit_needs_an_exception() {
    let files = [
        file("crates/a/src/big.rs", 11, &[("run", 6)]),
        file("crates/a/tests/suite.rs", 21, &[]),
    ];
    let violations = evaluate(&policy_with(Vec::new()), &files);
    assert_eq!(violations.len(), 3, "{violations:#?}");
    assert!(violations[0].contains("10-line production file limit"));
    assert!(violations[1].contains("`run`") && violations[1].contains("production function"));
    assert!(violations[2].contains("20-line test file limit"));
}

#[test]
fn an_exception_allows_its_ceiling_and_fails_growth_past_it() {
    let policy = policy_with(vec![
        exception("crates/a/src/big.rs", None, 12),
        exception("crates/a/src/big.rs", Some("run"), 7),
    ]);
    let at_ceiling = [file("crates/a/src/big.rs", 12, &[("run", 7)])];
    assert!(evaluate(&policy, &at_ceiling).is_empty());

    let shrunk = [file("crates/a/src/big.rs", 11, &[("run", 6)])];
    assert!(evaluate(&policy, &shrunk).is_empty());

    let grown = [file("crates/a/src/big.rs", 13, &[("run", 8)])];
    let violations = evaluate(&policy, &grown);
    assert_eq!(violations.len(), 2, "{violations:#?}");
    assert!(violations
        .iter()
        .all(|violation| violation.contains("grew past its reviewed ceiling")));
}

#[test]
fn exceptions_that_no_longer_apply_must_be_removed() {
    let policy = policy_with(vec![
        exception("crates/a/src/fixed.rs", None, 12),
        exception("crates/a/src/fixed.rs", Some("run"), 7),
        exception("crates/a/src/gone.rs", None, 12),
        exception("crates/a/src/fixed.rs", Some("removed"), 7),
    ]);
    let files = [file("crates/a/src/fixed.rs", 10, &[("run", 5)])];
    let violations = evaluate(&policy, &files);
    assert_eq!(violations.len(), 4, "{violations:#?}");
    assert_eq!(
        violations
            .iter()
            .filter(|violation| violation.contains("remove its exception"))
            .count(),
        4
    );
}

#[test]
fn test_file_functions_are_not_limited() {
    let policy = policy_with(vec![exception("crates/a/src/tests.rs", Some("long"), 50)]);
    let files = [file("crates/a/src/tests.rs", 20, &[("long", 50)])];
    let violations = evaluate(&policy, &files);
    assert_eq!(violations.len(), 1, "{violations:#?}");
    assert!(violations[0].contains("no longer exists as a measured production module"));
}

#[test]
fn exceptions_must_be_unique_and_explained() {
    let mut unexplained = exception("crates/a/src/big.rs", None, 12);
    unexplained.reason = " ".to_string();
    let policy = policy_with(vec![
        exception("crates/a/src/big.rs", None, 12),
        unexplained,
    ]);
    let files = [file("crates/a/src/big.rs", 12, &[])];
    let violations = evaluate(&policy, &files);
    assert!(violations
        .iter()
        .any(|violation| violation.starts_with("duplicate module size exception")));
    assert!(violations
        .iter()
        .any(|violation| violation.starts_with("module size exception has no reason")));
}

#[test]
fn the_repository_policy_parses() {
    let policy = policy();
    assert_eq!(policy.limits.production_file, 600);
    assert_eq!(policy.limits.test_file, 900);
    assert_eq!(policy.limits.production_function, 200);
}
