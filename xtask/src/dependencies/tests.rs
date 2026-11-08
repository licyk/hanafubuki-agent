use super::*;

/// Metadata shaped as `cargo metadata --no-deps` prints it, keeping only a
/// few of the fields the check ignores.
fn metadata(packages: &[(&str, &[&str])]) -> Metadata {
    let packages: Vec<serde_json::Value> = packages
        .iter()
        .map(|(name, dependencies)| {
            serde_json::json!({
                "name": name,
                "version": "0.1.0",
                "id": format!("path+file:///repo/crates/{name}#0.1.0"),
                "dependencies": dependencies
                    .iter()
                    .map(|dependency| serde_json::json!({
                        "name": dependency,
                        "source": null,
                        "req": "*",
                        "kind": null,
                        "rename": null,
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let json = serde_json::json!({ "packages": packages, "workspace_members": [], "version": 1 });
    serde_json::from_value(json).unwrap()
}

#[test]
fn allows_hanafubuki_named_members_and_other_crates() {
    let metadata = metadata(&[
        ("hanafubuki-agent", &["hanafubuki-agent-kernel", "serde"]),
        ("hanafubuki-agent-kernel", &["tokio"]),
        ("xtask", &["syn"]),
    ]);
    assert_eq!(violations(&metadata, &[]), Vec::<String>::new());
}

#[test]
fn refuses_hanafubuki_crates_that_are_not_members() {
    let metadata = metadata(&[
        ("hanafubuki-agent", &["hanafubuki-foundation", "serde"]),
        ("xtask", &["hanafubuki"]),
    ]);
    assert_eq!(
        violations(&metadata, &[]),
        [
            "hanafubuki-agent depends on hanafubuki-foundation, a Hanafubuki crate: this repository depends on no Hanafubuki crate",
            "xtask depends on hanafubuki, a Hanafubuki crate: this repository depends on no Hanafubuki crate",
        ]
    );
}

#[test]
fn an_empty_layer_table_allows_any_internal_dependency() {
    let metadata = metadata(&[("low", &["high"]), ("high", &["low"])]);
    assert!(violations(&metadata, &[]).is_empty());
}

#[test]
fn enforces_the_layer_table_once_it_has_rows() {
    let metadata = metadata(&[
        ("low", &["serde"]),
        ("high", &["low", "serde"]),
        ("upside-down", &["high"]),
    ]);
    let layers: &[(&str, &[&str])] = &[("low", &[]), ("high", &["low"]), ("upside-down", &["low"])];
    assert_eq!(
        violations(&metadata, layers),
        ["upside-down depends on high, which its layer does not allow"]
    );
}

#[test]
fn a_layer_table_must_name_every_member_and_only_members() {
    let metadata = metadata(&[("low", &[]), ("unlisted", &["low"])]);
    let layers: &[(&str, &[&str])] = &[("low", &[]), ("gone", &["low"])];
    assert_eq!(
        violations(&metadata, layers),
        [
            "the layer table names gone, which is not a workspace member",
            "unlisted has no row in the layer table: add it with the members it may depend on",
        ]
    );
}
