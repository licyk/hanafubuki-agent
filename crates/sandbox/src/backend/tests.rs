use super::{Sandbox, SandboxError, SandboxStatus};
use crate::SandboxPolicy;

fn command() -> Vec<String> {
    vec!["/bin/sh".to_string(), "-c".to_string(), "true".to_string()]
}

#[test]
fn full_access_leaves_the_command_as_it_is_whatever_the_machine_has() {
    for sandbox in [Sandbox::detect(), Sandbox::unavailable("none here")] {
        assert_eq!(
            sandbox.wrap(&SandboxPolicy::FullAccess, command()),
            Ok(command())
        );
    }
}

#[test]
fn a_policy_no_backend_enforces_fails_closed_and_says_why() {
    let sandbox = Sandbox::unavailable("simulated");
    assert_eq!(
        sandbox.status(),
        SandboxStatus::Unavailable {
            reason: "simulated".to_string()
        }
    );
    for policy in [
        SandboxPolicy::ReadOnly,
        SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: true,
        },
    ] {
        let error = sandbox.wrap(&policy, command()).unwrap_err();
        assert_eq!(
            error,
            SandboxError::Unavailable {
                mode: policy.mode(),
                reason: "simulated".to_string()
            }
        );
        assert!(error.to_string().contains("full-access"), "{error}");
    }
}

// macOS's Seatbelt needs no helper.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_test_binary_that_never_dispatched_has_no_helper() {
    assert!(matches!(
        Sandbox::detect().status(),
        SandboxStatus::Unavailable { .. }
    ));
}

#[test]
fn seatbelt_runs_a_sandboxed_command_under_sandbox_exec() {
    let sandbox = Sandbox {
        backend: super::Backend::Seatbelt,
    };
    assert_eq!(
        sandbox.status(),
        SandboxStatus::Available {
            backend: "Seatbelt (/usr/bin/sandbox-exec)".to_string()
        }
    );
    for policy in [
        SandboxPolicy::ReadOnly,
        SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
        },
    ] {
        let wrapped = sandbox.wrap(&policy, command()).unwrap();
        assert_eq!(wrapped[..2], ["/usr/bin/sandbox-exec", "-p"]);
        assert_eq!(wrapped[3..], ["--", "/bin/sh", "-c", "true"]);
    }
    assert_eq!(
        sandbox.wrap(&SandboxPolicy::FullAccess, command()),
        Ok(command())
    );
}

#[test]
fn the_windows_sandbox_setting_chooses_a_backend_only_on_windows() {
    let available = Sandbox {
        backend: super::Backend::Helper {
            helper: "/agent".into(),
            confinement: "a restricted token".to_string(),
        },
    };
    for setting in [None, Some(crate::AgentWindowsSandbox::Elevated)] {
        let chosen = available.with_windows_sandbox(setting);
        assert_eq!(
            matches!(chosen.status(), SandboxStatus::Unavailable { .. }),
            cfg!(windows),
            "{setting:?}"
        );
    }
    assert_eq!(
        available.with_windows_sandbox(Some(crate::AgentWindowsSandbox::Unelevated)),
        available
    );
    let unavailable = Sandbox::unavailable("none here");
    assert_eq!(
        unavailable.with_windows_sandbox(Some(crate::AgentWindowsSandbox::Unelevated)),
        unavailable
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_kernel_without_landlock_has_no_sandbox() {
    for abi in [-1, 0] {
        let backend = super::linux_backend("/agent".into(), abi);
        let sandbox = Sandbox { backend };
        let error = sandbox
            .wrap(&SandboxPolicy::ReadOnly, command())
            .unwrap_err();
        assert!(
            matches!(&error, SandboxError::Unavailable { mode: crate::AgentSandboxMode::ReadOnly, reason } if reason.contains("Landlock")),
            "{error:?}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_sandboxed_command_reexecutes_the_helper() {
    let sandbox = Sandbox {
        backend: super::linux_backend("/agent".into(), 1),
    };
    let wrapped = sandbox
        .wrap(
            &SandboxPolicy::WorkspaceWrite {
                writable_roots: vec!["/work".into()],
                network_access: false,
            },
            command(),
        )
        .unwrap();
    assert_eq!(
        wrapped,
        [
            "/agent",
            crate::HELPER_ARG,
            "--writable-root",
            "/work",
            "--",
            "/bin/sh",
            "-c",
            "true"
        ]
    );
}
