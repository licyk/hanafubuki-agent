use std::collections::BTreeSet;

use hanafubuki_agent_config::{
    config_sections, AgentSettings, AgentStandaloneSettings, SettingKind, SettingSpec,
};
use serde_json::{json, Map, Value};

use super::{set, setup, write};

/// The tables keyed by what the user added: collections, without specs.
const COLLECTIONS: [&str; 5] = [
    "plugins",
    "marketplaces",
    "pluginOptions",
    "tui.keymap",
    "tui.presets",
];

fn defaults() -> Map<String, Value> {
    let mut defaults = object(serde_json::to_value(AgentSettings::default()).unwrap());
    defaults.extend(object(
        serde_json::to_value(AgentStandaloneSettings::default()).unwrap(),
    ));
    defaults
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => panic!("not an object: {other}"),
    }
}

/// Every serialized leaf of the defaults, by dotted key, but the
/// collections'.
fn leaves(table: &Map<String, Value>, prefix: &str, found: &mut Vec<(String, Value)>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if COLLECTIONS.contains(&path.as_str()) {
            continue;
        }
        match value {
            Value::Object(nested) if !nested.is_empty() => leaves(nested, &path, found),
            value => found.push((path, value.clone())),
        }
    }
}

fn specs() -> Vec<SettingSpec> {
    config_sections()
        .into_iter()
        .flat_map(|section| section.settings)
        .collect()
}

#[test]
fn every_leaf_of_the_defaults_has_exactly_one_spec_and_every_spec_names_a_leaf() {
    let mut found = Vec::new();
    leaves(&defaults(), "", &mut found);
    let leaves: BTreeSet<String> = found.into_iter().map(|(key, _)| key).collect();
    let specs = specs();
    let keys: BTreeSet<String> = specs.iter().map(|spec| spec.key.clone()).collect();
    assert_eq!(keys.len(), specs.len(), "a key has two specs");
    let missing: Vec<_> = leaves.difference(&keys).collect();
    let extra: Vec<_> = keys.difference(&leaves).collect();
    assert!(missing.is_empty(), "leaves without a spec: {missing:?}");
    assert!(extra.is_empty(), "specs naming no leaf: {extra:?}");

    let sections = config_sections();
    let ids: BTreeSet<&str> = sections.iter().map(|section| section.id.as_str()).collect();
    assert_eq!(ids.len(), sections.len(), "two sections share an id");
}

#[test]
fn every_default_comes_from_the_structs_and_passes_its_spec() {
    let mut found = Vec::new();
    leaves(&defaults(), "", &mut found);
    for spec in specs() {
        let (_, value) = found.iter().find(|(key, _)| *key == spec.key).unwrap();
        match &spec.default {
            None => assert!(value.is_null(), "{} has no default but {value}", spec.key),
            Some(default) => {
                assert_eq!(&default.to_json(), value, "{}", spec.key);
                spec.check(value)
                    .unwrap_or_else(|error| panic!("{}'s default: {error}", spec.key));
            }
        }
    }
}

#[test]
fn every_value_of_a_choice_is_written_and_read_back() {
    // SearXNG needs an endpoint before it can be chosen.
    let fixture = setup("searxngBaseUrl = \"https://search.example.test\"\n", "");
    let config = fixture.layered(None, &[]);
    for spec in specs() {
        let SettingKind::Choice { values } = &spec.kind else {
            continue;
        };
        for choice in values.iter().filter(|choice| choice.disabled.is_none()) {
            let value = json!(choice.value);
            config
                .write_values(&write(vec![set(&spec.key, value.clone())]))
                .unwrap_or_else(|error| panic!("{} = {value}: {error}", spec.key));
            assert_eq!(
                config.value(&spec.key).unwrap(),
                Some(value),
                "{}",
                spec.key
            );
        }
    }
}
