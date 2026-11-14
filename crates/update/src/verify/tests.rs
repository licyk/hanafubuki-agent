use super::*;

#[test]
fn only_the_programs_name_and_one_version_is_an_answer() {
    assert_eq!(
        reported_version("hanafubuki-agent 0.3.0\n").as_deref(),
        Some("0.3.0")
    );
    assert_eq!(
        reported_version("  hanafubuki-agent 0.3.0+local \r\n").as_deref(),
        Some("0.3.0+local")
    );
    for output in [
        "",
        "hanafubuki-agent",
        "hanafubuki-agent ",
        "hanamori 0.3.0",
        "hanafubuki-agent-cli 0.3.0",
        "hanafubuki-agent  0.3.0",
        "hanafubuki-agent 0.3.0\nmore",
    ] {
        assert_eq!(reported_version(output), None, "{output:?}");
    }
}

#[test]
fn what_an_executable_prints_is_escaped_and_cut() {
    assert_eq!(quoted("a\u{1b}[31mred"), "\"a\\u{1b}[31mred\"");
    assert_eq!(first_line("first\nsecond"), "first");
    let long = "x".repeat(500);
    assert_eq!(escaped(&long).chars().count(), 201);
}

#[cfg(unix)]
mod scripts {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Instant;

    use super::*;

    const EXPECTED: Version = Version::new(9, 9, 9);

    fn script(directory: &tempfile::TempDir, body: &str) -> PathBuf {
        let path = directory.path().join("hanafubuki-agent");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    async fn check(body: &str, timeout: Duration) -> Result<(), UpdateError> {
        let directory = tempfile::tempdir().unwrap();
        check_staged(&script(&directory, body), &EXPECTED, timeout).await
    }

    #[tokio::test]
    async fn the_staged_executable_must_print_its_version_exactly() {
        check("echo 'hanafubuki-agent 9.9.9'", Duration::from_secs(10))
            .await
            .unwrap();
        let error = check("echo 'hanafubuki-agent 9.9.8'", Duration::from_secs(10))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            UpdateError::WrongVersion {
                reported: "\"hanafubuki-agent 9.9.8\"".to_string(),
                expected: EXPECTED,
            }
        );
        assert_eq!(
            error.to_string(),
            "the new executable reports \"hanafubuki-agent 9.9.8\", not hanafubuki-agent 9.9.9; \
             nothing was changed."
        );
    }

    #[tokio::test]
    async fn a_failing_executable_is_reported_by_its_first_line_of_errors() {
        let error = check(
            "echo \"version 'GLIBC_2.35' not found\" >&2; echo more >&2; exit 1",
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            UpdateError::NotRunnable {
                reason: "version 'GLIBC_2.35' not found".to_string(),
                current: crate::current_version(),
            }
        );
        assert_eq!(
            error
                .to_string()
                .contains("The Linux build needs glibc 2.35 or newer."),
            cfg!(target_os = "linux")
        );
        let silent = check("exit 3", Duration::from_secs(10)).await.unwrap_err();
        assert!(
            matches!(&silent, UpdateError::NotRunnable { reason, .. } if reason.contains('3')),
            "{silent:?}"
        );
    }

    #[tokio::test]
    async fn a_file_that_does_not_start_is_not_runnable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hanafubuki-agent");
        std::fs::write(&path, [0x7f, b'E', b'L', b'F', 0, 0, 0, 0]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = check_staged(&path, &EXPECTED, Duration::from_secs(10))
            .await
            .unwrap_err();
        assert!(
            matches!(error, UpdateError::NotRunnable { .. }),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_silent_executable_is_killed_at_the_bound() {
        let started = Instant::now();
        let error = check("exec sleep 30", Duration::from_millis(300))
            .await
            .unwrap_err();
        assert_eq!(error, UpdateError::VersionTimeout { seconds: 0 });
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(
            UpdateError::VersionTimeout { seconds: 10 }.to_string(),
            "the new executable did not answer --version within 10 s; nothing was changed."
        );
    }

    #[tokio::test]
    async fn the_installed_version_is_read_or_not_at_all() {
        let directory = tempfile::tempdir().unwrap();
        let timeout = Duration::from_secs(10);
        let path = script(&directory, "echo 'hanafubuki-agent 9.9.9'");
        assert_eq!(installed_version(&path, timeout).await, Some(EXPECTED));
        for body in [
            "echo junk",
            "echo 'hanafubuki-agent 9.9.9'; exit 1",
            "exit 0",
        ] {
            let path = script(&directory, body);
            assert_eq!(installed_version(&path, timeout).await, None, "{body}");
        }
        assert_eq!(
            installed_version(&directory.path().join("missing"), timeout).await,
            None
        );
    }
}
