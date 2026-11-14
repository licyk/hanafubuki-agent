use super::*;

#[test]
fn protected_paths_are_mounted_after_all_writable_roots() {
    let workspace = tempfile::tempdir().unwrap();
    let git = workspace.path().join(".git");
    std::fs::create_dir(&git).unwrap();
    let request = HelperRequest {
        writable_roots: vec![git.clone(), workspace.path().to_path_buf()],
        readable_roots: None,
        network_access: false,
        command: vec!["/bin/true".into()],
    };
    let argv = arguments(Path::new("/bin/true"), &request).unwrap();
    let last_write = argv.iter().rposition(|arg| arg == "--bind").unwrap();
    let protect = argv
        .windows(3)
        .rposition(|args| args[0] == "--ro-bind" && args[1] == git.as_os_str())
        .unwrap();
    assert!(protect > last_write);
    assert!(argv.iter().any(|arg| arg == "--unshare-net"));
    assert!(argv.iter().any(|arg| arg == "--network-only"));
}

#[test]
fn network_access_and_plugin_reads_keep_separate_boundaries() {
    let package = tempfile::tempdir().unwrap();
    let request = HelperRequest {
        writable_roots: Vec::new(),
        readable_roots: Some(vec![package.path().into()]),
        network_access: true,
        command: vec!["/bin/true".into()],
    };
    let argv = arguments(Path::new("/bin/true"), &request).unwrap();
    assert!(argv.windows(2).any(|args| args == ["--tmpfs", "/"]));
    assert!(!argv.iter().any(|arg| arg == "--unshare-net"));
    assert!(!argv.windows(3).any(|args| args == ["--ro-bind", "/", "/"]));
}
