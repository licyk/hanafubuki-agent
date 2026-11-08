use std::path::PathBuf;

use super::*;

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hanafubuki-workspace-roots-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn a_single_root_is_its_own_primary() {
    let root = temp_root("single");
    let roots = agent_workspace_roots_from([root.clone()]).unwrap();
    assert_eq!(roots.primary(), &root);
    assert_eq!(roots.roots(), [root.clone()]);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_empty_root_set_is_not_a_workspace() {
    assert!(agent_workspace_roots_from([]).is_none());
}

/// A managed instance already lives under the managed root, so listing it again
/// would only make every containment check longer.
#[test]
fn a_root_nested_in_an_earlier_root_is_dropped() {
    let managed = temp_root("nested");
    let child = managed.join("instance-a");
    std::fs::create_dir_all(&child).unwrap();

    let roots = agent_workspace_roots_from([managed.clone(), child]).unwrap();

    assert_eq!(roots.roots(), [managed.clone()]);
    std::fs::remove_dir_all(&managed).unwrap();
}

/// An attached instance on another volume is exactly the case multi-root exists
/// for: it is not under the managed root and must not need elevation.
#[test]
fn a_sibling_root_outside_the_managed_root_is_kept() {
    let managed = temp_root("sibling-managed");
    let attached = temp_root("sibling-attached");

    let roots = agent_workspace_roots_from([managed.clone(), attached.clone()]).unwrap();

    assert_eq!(roots.roots().len(), 2);
    assert_eq!(roots.primary(), &managed);
    assert!(roots.roots().contains(&attached));

    std::fs::remove_dir_all(&managed).unwrap();
    std::fs::remove_dir_all(&attached).unwrap();
}

/// The managed root stays first whatever order the instances arrive in, because
/// it is what relative requests resolve against.
#[test]
fn the_first_root_supplied_remains_the_primary() {
    let managed = temp_root("primary-managed");
    let attached = temp_root("primary-attached");

    let roots = agent_workspace_roots_from([managed.clone(), attached]).unwrap();

    assert_eq!(roots.primary(), &managed);

    std::fs::remove_dir_all(&managed).unwrap();
    std::fs::remove_dir_all(roots.roots()[1].as_path()).unwrap();
}

/// The same directory reached by two spellings is one root. On Windows those
/// two spellings also differ in case and prefix kind.
#[test]
fn one_location_spelled_twice_is_one_root() {
    let managed = temp_root("duplicate");
    let nested = managed.join("child");
    std::fs::create_dir_all(&nested).unwrap();

    let roots =
        agent_workspace_roots_from([managed.clone(), managed.join("child").join("..")]).unwrap();

    assert_eq!(roots.roots().len(), 1);
    std::fs::remove_dir_all(&managed).unwrap();
}
