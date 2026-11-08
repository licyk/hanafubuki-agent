use std::ffi::OsStr;
use std::path::Path;

use crate::paths::{PathError, ResolvedPath};

fn temp_root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hanafubuki-resolved-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("workspace").join("nested")).unwrap();
    std::fs::write(root.join("workspace").join("file.txt"), b"content").unwrap();
    root
}

#[test]
fn resolving_yields_three_agreeing_forms() {
    let root = temp_root();
    let resolved = ResolvedPath::resolve(root.join("workspace").join("file.txt")).unwrap();

    assert!(resolved.native().is_absolute());
    assert_eq!(resolved.native_text(), resolved.native().to_str().unwrap());
    assert_eq!(
        resolved.key(),
        &crate::paths::PathKey::of(resolved.native())
    );
    // The system form is what the OS returned and is never shown; the native
    // form is what a reader gets. Both must name the same file.
    assert!(std::fs::read(resolved.system()).is_ok());

    std::fs::remove_dir_all(&root).unwrap();
}

/// The form handed to a model, to the interface, or to a record must never
/// carry a Windows verbatim or device prefix.
#[test]
fn the_interchange_form_is_never_verbatim() {
    let root = temp_root();
    let resolved = ResolvedPath::resolve(root.join("workspace")).unwrap();
    assert!(!resolved.native_text().starts_with(r"\\?\"));
    assert!(!resolved.native_text().starts_with(r"\\.\"));
    std::fs::remove_dir_all(&root).unwrap();
}

/// The reported Windows failure: a relative request containing `..` resolved
/// against a verbatim base produced a string Win32 refuses to open. Collapsing
/// before touching the filesystem is what fixes it.
#[test]
fn relative_parent_traversal_resolves_against_a_resolved_base() {
    let root = temp_root();
    let base = ResolvedPath::resolve(root.join("workspace").join("nested")).unwrap();

    let requested = base.join_request(Path::new("../file.txt")).unwrap();
    let resolved = ResolvedPath::resolve(&requested).unwrap();

    assert_eq!(resolved.file_name().unwrap(), OsStr::new("file.txt"));
    assert!(resolved.starts_with(&ResolvedPath::resolve(root.join("workspace")).unwrap()));

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_absolute_request_ignores_the_base() {
    let root = temp_root();
    let base = ResolvedPath::resolve(root.join("workspace").join("nested")).unwrap();
    let target = root.join("workspace").join("file.txt");

    let requested = base.join_request(&target).unwrap();
    assert_eq!(
        ResolvedPath::resolve(requested).unwrap(),
        ResolvedPath::resolve(target).unwrap()
    );

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_missing_child_inherits_the_resolved_parent() {
    let root = temp_root();
    let parent = ResolvedPath::resolve(root.join("workspace")).unwrap();
    let child = ResolvedPath::resolve_child(&parent, OsStr::new("created.txt")).unwrap();

    assert!(child.starts_with(&parent));
    assert!(child.native_text().ends_with("created.txt"));
    // Writing through the system form must land at the native form.
    std::fs::write(child.system(), b"x").unwrap();
    assert_eq!(
        ResolvedPath::resolve(child.native()).unwrap().key(),
        child.key()
    );

    std::fs::remove_dir_all(&root).unwrap();
}

/// A traversal hidden in the final component must not be accepted as a name.
#[test]
fn a_missing_child_must_be_a_single_ordinary_component() {
    let root = temp_root();
    let parent = ResolvedPath::resolve(root.join("workspace")).unwrap();

    for smuggled in ["..", ".", "nested/deeper.txt", ""] {
        assert!(
            matches!(
                ResolvedPath::resolve_child(&parent, OsStr::new(smuggled)),
                Err(PathError::InvalidComponent(_))
            ),
            "{smuggled:?} was accepted as a file name"
        );
    }

    std::fs::remove_dir_all(&root).unwrap();
}

/// A caller that retries a failed resolution as a create must be able to tell
/// "does not exist" from "exists but could not be opened".
#[test]
fn a_missing_path_is_distinguishable_from_other_failures() {
    let root = temp_root();
    let error = ResolvedPath::resolve(root.join("workspace").join("absent.txt")).unwrap_err();
    assert!(error.is_not_found());
    assert!(ResolvedPath::resolve("").unwrap_err() == PathError::Empty);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn equality_is_identity_not_spelling() {
    let root = temp_root();
    let direct = ResolvedPath::resolve(root.join("workspace")).unwrap();
    let indirect = ResolvedPath::resolve(root.join("workspace").join("nested").join("..")).unwrap();
    assert_eq!(direct, indirect);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_deleted_location_no_longer_resolves_here() {
    let root = temp_root();
    let resolved = ResolvedPath::resolve(root.join("workspace").join("file.txt")).unwrap();
    assert!(resolved.still_resolves_here());
    std::fs::remove_file(resolved.system()).unwrap();
    assert!(!resolved.still_resolves_here());
    std::fs::remove_dir_all(&root).unwrap();
}
