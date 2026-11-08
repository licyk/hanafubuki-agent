use super::*;

mod key;
mod native;
mod resolved;

#[test]
fn rejects_empty_paths() {
    assert!(normalize_path(PathBuf::new()).is_err());
}

#[test]
fn normalizes_lexical_parent_components() {
    let normalized = normalize_path(PathBuf::from("alpha/beta/../gamma")).unwrap();
    assert!(normalized.ends_with("alpha/gamma"));
    assert!(normalized.is_absolute());
}

/// A `..` run longer than the path is a malformed request, not a licence to
/// walk off the top of the volume. The root is spelled differently per
/// platform: POSIX is `/`, Windows is a drive-qualified `C:\`.
#[test]
fn parent_components_never_pop_past_the_root() {
    #[cfg(not(windows))]
    let (input, expected) = ("/alpha/../../../beta", "/beta");
    #[cfg(windows)]
    let (input, expected) = (r"C:\alpha\..\..\..\beta", r"C:\beta");
    let normalized = normalize_path(PathBuf::from(input)).unwrap();
    assert_eq!(normalized, PathBuf::from(expected));
}

#[cfg(windows)]
#[test]
fn converts_verbatim_disk_and_unc_paths_to_native_paths() {
    assert_eq!(
        normalize_path(PathBuf::from(r"\\?\C:\Users\developer\Models")).unwrap(),
        PathBuf::from(r"C:\Users\developer\Models")
    );
    assert_eq!(
        normalize_path(PathBuf::from(r"\\?\UNC\server\share\Models")).unwrap(),
        PathBuf::from(r"\\server\share\Models")
    );
}

#[cfg(windows)]
#[test]
fn rejects_unsupported_windows_device_namespaces() {
    assert!(normalize_path(PathBuf::from(r"\\.\PhysicalDrive0")).is_err());
    assert!(normalize_path(PathBuf::from(
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\"
    ))
    .is_err());
}
