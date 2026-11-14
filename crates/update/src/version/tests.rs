use super::*;

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn the_plans_table_of_versions() {
    for (current, latest, newer) in [
        ("0.2.0", "0.3.0", true),
        ("0.2.0", "0.2.0", false),
        ("0.2.0+local", "0.2.0", false),
        ("0.3.0-rc.1", "0.3.0", true),
        ("0.3.0", "0.2.1", false),
        ("0.2.0", "0.3.0-beta.1", false),
        ("0.9.9", "1.0.0", true),
        ("1.0.0", "0.9.9", false),
        ("0.2.0", "0.2.1+build.7", true),
    ] {
        assert_eq!(
            is_newer(&version(latest), &version(current)),
            newer,
            "{latest} against {current}"
        );
    }
}

#[test]
fn build_metadata_never_makes_a_release_newer() {
    // semver's `Ord` orders build metadata; precedence does not.
    assert!(version("0.2.0+b") > version("0.2.0+a"));
    assert!(!is_newer(&version("0.2.0+b"), &version("0.2.0+a")));
}

#[test]
fn a_release_tag_is_v_and_a_stable_version() {
    assert_eq!(release_version("v0.3.0"), Ok(version("0.3.0")));
    assert_eq!(release_version(" v0.3.0\n"), Ok(version("0.3.0")));
    assert_eq!(release_version("v1.2.3+build"), Ok(version("1.2.3+build")));
    assert_eq!(
        release_version("v0.3.0-beta.1"),
        Err(TagProblem::PreRelease)
    );
    for tag in [
        "nightly", "0.3.0", "v", "v0.3", "v0.3.0.1", "V0.3.0", "vv0.3.0", "v 0.3.0", "v01.2.3", "",
    ] {
        assert_eq!(
            release_version(tag),
            Err(TagProblem::NotAVersion),
            "{tag:?}"
        );
    }
}

#[test]
fn the_current_version_is_the_workspaces() {
    assert_eq!(current_version().to_string(), env!("CARGO_PKG_VERSION"));
}
