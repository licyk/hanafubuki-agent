use std::path::PathBuf;

use hanafubuki_agent_foundation::agent_settings::AgentSandboxMode;

use super::{SandboxPolicy, SandboxSettings};

fn settings(mode: AgentSandboxMode) -> SandboxSettings {
    SandboxSettings {
        mode,
        writable_roots: vec![PathBuf::from("/data"), PathBuf::from("/work")],
        network_access: true,
    }
}

#[test]
fn workspace_write_writes_the_workspace_the_configured_roots_and_the_temporary_directories() {
    let policy = settings(AgentSandboxMode::WorkspaceWrite).policy(&[PathBuf::from("/work")]);
    let SandboxPolicy::WorkspaceWrite {
        writable_roots,
        network_access,
    } = policy
    else {
        panic!("{policy:?}");
    };
    assert!(network_access);
    assert_eq!(
        writable_roots[..2],
        [PathBuf::from("/work"), PathBuf::from("/data")]
    );
    if cfg!(unix) {
        assert!(writable_roots.contains(&PathBuf::from("/tmp")));
        assert!(writable_roots.contains(&std::env::temp_dir()));
    }
    let unique: std::collections::BTreeSet<_> = writable_roots.iter().collect();
    assert_eq!(unique.len(), writable_roots.len());
}

#[test]
fn read_only_and_full_access_ignore_the_workspace_write_settings() {
    let roots = [PathBuf::from("/work")];
    assert_eq!(
        settings(AgentSandboxMode::ReadOnly).policy(&roots),
        SandboxPolicy::ReadOnly
    );
    assert_eq!(
        settings(AgentSandboxMode::FullAccess).policy(&roots),
        SandboxPolicy::FullAccess
    );
}
