use super::*;

use std::collections::HashMap;

#[test]
fn child_process_environment_replaces_one_atomic_snapshot() {
    let environment = ChildProcessEnvironment::default();
    environment.replace(HashMap::from([
        (
            "HTTP_PROXY".to_string(),
            Some("http://proxy:8080".to_string()),
        ),
        ("HTTPS_PROXY".to_string(), None),
        ("=C:".to_string(), Some(r"C:\Users\example".to_string())),
    ]));
    assert_eq!(
        environment.snapshot(),
        HashMap::from([
            (
                "HTTP_PROXY".to_string(),
                Some("http://proxy:8080".to_string())
            ),
            ("HTTPS_PROXY".to_string(), None),
            ("=C:".to_string(), Some(r"C:\Users\example".to_string())),
        ])
    );

    environment.replace(HashMap::from([(
        "NO_PROXY".to_string(),
        Some("localhost".to_string()),
    )]));
    assert_eq!(
        environment.snapshot(),
        HashMap::from([("NO_PROXY".to_string(), Some("localhost".to_string()))])
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
    ]));
    environment.replace(HashMap::from([(
        "HTTP_PROXY".to_string(),
        Some("http://proxy:8080".to_string()),
    )]));

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
