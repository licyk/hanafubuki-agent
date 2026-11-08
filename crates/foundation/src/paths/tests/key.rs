//! `PathKey` is the contract that makes containment checks correct on Windows.
//! The Windows cases are compiled only there, but the folding rules they assert
//! are the reason the type exists at all.

use std::path::Path;

use std::path::PathBuf;

use crate::paths::{relative_below, PathKey};

#[test]
fn containment_is_component_wise_not_textual() {
    let base = PathKey::of(Path::new("/workspace/models"));
    assert!(PathKey::of(Path::new("/workspace/models/sd/model.safetensors")).starts_with(&base));
    assert!(PathKey::of(Path::new("/workspace/models")).starts_with(&base));
    // The textual prefix matches; the location is a sibling.
    assert!(!PathKey::of(Path::new("/workspace/models-extra/x")).starts_with(&base));
}

#[test]
fn relative_below_returns_the_tail_only_for_contained_paths() {
    let base = Path::new("/data/app");
    assert_eq!(
        relative_below(Path::new("/data/app/config/language.json"), base),
        Some(PathBuf::from("config/language.json"))
    );
    assert_eq!(relative_below(base, base), Some(PathBuf::new()));
    assert_eq!(relative_below(Path::new("/data/app-other/x"), base), None);
}

#[cfg(windows)]
#[test]
fn relative_below_ignores_case_and_verbatim_spelling() {
    assert_eq!(
        relative_below(
            Path::new(r"\\?\C:\Users\Me\AppData\config\a.json"),
            Path::new(r"c:\users\me\appdata"),
        ),
        Some(PathBuf::from(r"config\a.json"))
    );
}

#[test]
fn distinct_locations_do_not_collide() {
    assert_ne!(
        PathKey::of(Path::new("/workspace/alpha")),
        PathKey::of(Path::new("/workspace/beta"))
    );
}

#[cfg(windows)]
#[test]
fn verbatim_and_native_disk_paths_share_one_identity() {
    assert_eq!(
        PathKey::of(Path::new(r"\\?\E:\Softwares\ComfyUI")),
        PathKey::of(Path::new(r"E:\Softwares\ComfyUI"))
    );
}

#[cfg(windows)]
#[test]
fn verbatim_and_native_unc_paths_share_one_identity() {
    assert_eq!(
        PathKey::of(Path::new(r"\\?\UNC\server\share\models")),
        PathKey::of(Path::new(r"\\server\share\models"))
    );
}

/// The reported failure mode: a protected root recorded in one spelling must
/// still block a request written in the other.
#[cfg(windows)]
#[test]
fn containment_folds_prefix_kind_and_case_together() {
    let base = PathKey::of(Path::new(r"e:\softwares\comfyui"));
    let child = PathKey::of(Path::new(
        r"\\?\E:\Softwares\ComfyUI\custom_nodes\zz_debug.py",
    ));
    assert!(child.starts_with(&base));
}

#[cfg(windows)]
#[test]
fn windows_identity_is_case_insensitive() {
    assert_eq!(
        PathKey::of(Path::new(r"C:\Users\Developer\Models")),
        PathKey::of(Path::new(r"c:\users\developer\models"))
    );
}

/// POSIX filesystems are case sensitive, and folding there would merge two real
/// files.
#[cfg(not(windows))]
#[test]
fn posix_identity_is_case_sensitive() {
    assert_ne!(
        PathKey::of(Path::new("/workspace/Models")),
        PathKey::of(Path::new("/workspace/models"))
    );
}
