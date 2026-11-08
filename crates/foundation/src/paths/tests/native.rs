//! The native/system round trip. Stripping `\\?\` unconditionally would be a
//! regression, not a fix: even with a `longPathAware` manifest, it collapses
//! the two path roles, so `system_form` must put the prefix back.

use std::path::PathBuf;

use crate::paths::{native_form, system_form};

#[test]
fn ordinary_paths_are_returned_unchanged_and_borrowed() {
    let path = PathBuf::from("/workspace/models/model.safetensors");
    assert_eq!(native_form(&path).unwrap(), path);
    assert!(matches!(system_form(&path), std::borrow::Cow::Borrowed(_)));
}

#[cfg(windows)]
#[test]
fn long_disk_paths_regain_the_verbatim_prefix() {
    let long = PathBuf::from(format!(r"C:\{}", "segment\\".repeat(40)));
    assert!(long.as_os_str().len() > 247);
    let system = system_form(&long);
    assert!(system.to_string_lossy().starts_with(r"\\?\C:\"));
    // Round trip: the system form resolves back to exactly the native form.
    assert_eq!(native_form(&system).unwrap(), long);
}

#[cfg(windows)]
#[test]
fn long_unc_paths_regain_the_verbatim_unc_prefix() {
    let long = PathBuf::from(format!(r"\\server\share\{}", "segment\\".repeat(40)));
    let system = system_form(&long);
    assert!(system
        .to_string_lossy()
        .starts_with(r"\\?\UNC\server\share\"));
    assert_eq!(native_form(&system).unwrap(), long);
}

/// Prefixing a path that still carries `..` would produce a string the object
/// manager cannot open, because Win32 does not normalize a verbatim path.
#[cfg(windows)]
#[test]
fn unnormalized_long_paths_are_left_alone() {
    let long = PathBuf::from(format!(r"C:\{}..\tail", "segment\\".repeat(40)));
    assert!(matches!(system_form(&long), std::borrow::Cow::Borrowed(_)));
}

#[cfg(windows)]
#[test]
fn short_paths_are_not_prefixed() {
    let path = PathBuf::from(r"E:\Softwares\ComfyUI");
    assert!(matches!(system_form(&path), std::borrow::Cow::Borrowed(_)));
}

#[cfg(windows)]
#[test]
fn device_namespaces_have_no_native_spelling() {
    assert!(native_form(std::path::Path::new(r"\\.\PhysicalDrive0")).is_err());
    assert!(native_form(std::path::Path::new(
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\"
    ))
    .is_err());
}
