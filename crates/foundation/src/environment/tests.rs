use super::*;

use std::collections::HashMap;

#[test]
fn accepts_native_non_shell_names() {
    assert!(validate_native_environment_entry("INTELLIJ IDEA", Some("value")).is_ok());
}

#[test]
fn rejects_unrepresentable_names_and_values() {
    assert!(validate_native_environment_entry("", Some("value")).is_err());
    assert!(validate_native_environment_entry("BAD=KEY", Some("value")).is_err());
    assert!(validate_native_environment_entry("BAD\0KEY", Some("value")).is_err());
    assert!(validate_native_environment_entry("KEY", Some("bad\0value")).is_err());
}

#[test]
fn child_process_environment_replaces_one_atomic_snapshot() {
    let environment = ChildProcessEnvironment::default();
    environment
        .replace(HashMap::from([
            (
                "HTTP_PROXY".to_string(),
                Some("http://proxy:8080".to_string()),
            ),
            ("HTTPS_PROXY".to_string(), None),
        ]))
        .unwrap();
    assert_eq!(
        environment.snapshot(),
        HashMap::from([
            (
                "HTTP_PROXY".to_string(),
                Some("http://proxy:8080".to_string())
            ),
            ("HTTPS_PROXY".to_string(), None),
        ])
    );

    environment
        .replace(HashMap::from([(
            "NO_PROXY".to_string(),
            Some("localhost".to_string()),
        )]))
        .unwrap();
    assert_eq!(
        environment.snapshot(),
        HashMap::from([("NO_PROXY".to_string(), Some("localhost".to_string()))])
    );
}

#[test]
fn child_process_environment_rejects_invalid_replacement_without_mutation() {
    let environment = ChildProcessEnvironment::default();
    environment
        .replace(HashMap::from([(
            "SAFE".to_string(),
            Some("value".to_string()),
        )]))
        .unwrap();

    assert!(environment
        .replace(HashMap::from([(
            "BAD=KEY".to_string(),
            Some("value".to_string()),
        )]))
        .is_err());
    assert_eq!(
        environment.snapshot(),
        HashMap::from([("SAFE".to_string(), Some("value".to_string()))])
    );
}

#[test]
fn child_process_environment_keeps_inherited_and_final_overlays_separate() {
    let environment = ChildProcessEnvironment::with_inherited_overlay(HashMap::from([
        ("LD_LIBRARY_PATH".to_string(), None),
        (
            "PATH".to_string(),
            Some("/home/user/bin:/usr/bin".to_string()),
        ),
    ]))
    .unwrap();
    environment
        .replace(HashMap::from([(
            "HTTP_PROXY".to_string(),
            Some("http://proxy:8080".to_string()),
        )]))
        .unwrap();

    assert_eq!(
        environment.inherited_snapshot(),
        HashMap::from([
            ("LD_LIBRARY_PATH".to_string(), None),
            (
                "PATH".to_string(),
                Some("/home/user/bin:/usr/bin".to_string())
            ),
        ])
    );
    assert_eq!(
        environment.snapshot(),
        HashMap::from([(
            "HTTP_PROXY".to_string(),
            Some("http://proxy:8080".to_string())
        )])
    );
}

#[test]
fn child_process_environment_rejects_invalid_inherited_overlay() {
    assert!(
        ChildProcessEnvironment::with_inherited_overlay(HashMap::from([(
            "BAD=KEY".to_string(),
            Some("value".to_string()),
        )]))
        .is_err()
    );
}
