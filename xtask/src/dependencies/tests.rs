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

/// The facade, the plugin API and two plugins, each with `extra` added to
/// its own dependencies.
fn plugin_workspace(extra: &[(&str, &str)]) -> Metadata {
    let mut packages: Vec<(&str, Vec<&str>)> = vec![
        (
            FACADE,
            vec![
                KERNEL,
                CONFIG,
                "hanafubuki-agent-plugin-a",
                "hanafubuki-agent-plugin-b",
            ],
        ),
        (KERNEL, vec![CONTRACTS, "kardia-core"]),
        (CONTRACTS, vec![HOST, "kardia-core"]),
        (HOST, vec![FOUNDATION, "kardia-core"]),
        (FOUNDATION, vec![]),
        (CONFIG, vec![FOUNDATION]),
        (MODEL_API, vec![FOUNDATION]),
        ("hanafubuki-agent-plugin-a", vec![CONTRACTS, "kardia-core"]),
        ("hanafubuki-agent-plugin-b", vec![CONTRACTS]),
    ];
    for (name, dependency) in extra {
        packages
            .iter_mut()
            .find(|(package, _)| package == name)
            .unwrap()
            .1
            .push(dependency);
    }
    let packages: Vec<(&str, &[&str])> = packages
        .iter()
        .map(|(name, dependencies)| (*name, dependencies.as_slice()))
        .collect();
    metadata(&packages)
}

#[test]
fn the_plugin_architecture_holds() {
    assert_eq!(
        rule_violations(&plugin_workspace(&[])),
        Vec::<String>::new()
    );
}

#[test]
fn a_plugin_reaches_the_agent_only_through_the_plugin_api() {
    let violations = rule_violations(&plugin_workspace(&[
        ("hanafubuki-agent-plugin-a", "hanafubuki-agent-plugin-b"),
        ("hanafubuki-agent-plugin-a", KERNEL),
        ("hanafubuki-agent-plugin-b", FACADE),
        (KERNEL, "hanafubuki-agent-plugin-b"),
    ]));
    assert_eq!(
        violations,
        [
            "hanafubuki-agent-kernel depends on the plugin hanafubuki-agent-plugin-b; plugins register through hanafubuki-agent-contracts",
            "hanafubuki-agent-plugin-a depends on hanafubuki-agent-kernel; a plugin reaches the Agent through hanafubuki-agent-contracts",
            "hanafubuki-agent-plugin-a depends on the plugin hanafubuki-agent-plugin-b; plugins share services through hanafubuki-agent-contracts",
            "hanafubuki-agent-plugin-b depends on hanafubuki-agent; a plugin reaches the Agent through hanafubuki-agent-contracts",
        ]
    );
}

#[test]
fn the_facade_alone_reads_the_configuration_and_mounts_every_plugin() {
    let metadata = metadata(&[
        (FACADE, &["hanafubuki-agent-plugin-a"]),
        (KERNEL, &[CONFIG]),
        (CONFIG, &[]),
        ("hanafubuki-agent-plugin-a", &[CONTRACTS]),
        ("hanafubuki-agent-plugin-b", &[CONTRACTS]),
        (CONTRACTS, &[]),
    ]);
    assert_eq!(
        rule_violations(&metadata),
        [
            "hanafubuki-agent does not depend on the plugin hanafubuki-agent-plugin-b; a plugin crate is mounted from the facade's manifest",
            "hanafubuki-agent-kernel depends on hanafubuki-agent-config; only the facade reads the configuration",
        ]
    );
}

#[test]
fn the_host_model_api_and_kardia_stay_in_their_place() {
    let violations = rule_violations(&plugin_workspace(&[
        (HOST, CONFIG),
        (MODEL_API, CONTRACTS),
        (MODEL_API, "kardia-macros"),
        (CONFIG, "kardia"),
    ]));
    assert_eq!(
        violations,
        [
            "hanafubuki-agent-host depends on hanafubuki-agent-config; only the facade reads the configuration",
            "hanafubuki-agent-host depends on hanafubuki-agent-config; the plugin host depends only on kardia and hanafubuki-agent-foundation",
            "hanafubuki-agent-config depends on the kardia facade; depend on kardia-core and kardia-macros",
            "hanafubuki-agent-config depends on kardia; only the plugin host, the plugin API, the kernel, the plugins and the facade may",

            "hanafubuki-agent-model-api depends on hanafubuki-agent-contracts; context sections and their renderers are the plugins' (check-model-api-context)",
            "hanafubuki-agent-model-api depends on kardia-macros; only the plugin host, the plugin API, the kernel, the plugins and the facade may",
        ]
    );
}

#[test]
fn only_the_guest_host_runs_kardias_loader_and_guests() {
    let metadata = metadata(&[
        (
            FACADE,
            &[GUEST_HOST, "hanafubuki-agent-plugin-a", "kardia-hmr"],
        ),
        (GUEST_HOST, &[CONTRACTS, "kardia-hmr", "kardia-loader"]),
        ("hanafubuki-agent-plugin-a", &[CONTRACTS, "kardia-loader"]),
        (CONTRACTS, &[]),
    ]);
    assert_eq!(
        rule_violations(&metadata),
        [
            "hanafubuki-agent depends on kardia-hmr; only the guest host (hanafubuki-agent-plugin-packages) may",
            "hanafubuki-agent-plugin-a depends on kardia-loader; only the guest host (hanafubuki-agent-plugin-packages) may",
        ]
    );
}
