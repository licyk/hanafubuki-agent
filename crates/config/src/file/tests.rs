use serde_json::json;

use super::*;

fn written(before: &str, value: Value) -> String {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(&path, before).unwrap();
    let file = ConfigFile::new(path.clone(), FileAccess::Shared);
    file.write(value.as_object().unwrap()).unwrap();
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_removed_value_leaves_the_document_and_a_new_one_joins_it() {
    let text = written(
        "keep = 1 # note\ngone = \"x\"\n\n[table]\n# above\nvalue = true\n",
        json!({"keep": 1, "gone": null, "added": "y", "table": {"value": false, "nested": {"deep": 2}}}),
    );
    let parsed: toml::Table = toml::from_str(&text).unwrap();
    assert!(parsed.get("gone").is_none());
    assert_eq!(parsed["added"].as_str(), Some("y"));
    assert_eq!(parsed["table"]["value"].as_bool(), Some(false));
    assert_eq!(parsed["table"]["nested"]["deep"].as_integer(), Some(2));
    assert!(text.contains("keep = 1 # note"), "{text}");
    assert!(text.contains("# above\nvalue = false"), "{text}");
}

#[test]
fn a_list_of_objects_is_an_array_of_tables() {
    let text = written(
        "",
        json!({"providers": [{"id": "a", "headers": [{"name": "x", "value": "y"}]}, {"id": "b", "headers": []}]}),
    );
    assert!(text.contains("[[providers]]"), "{text}");
    let parsed: Value = toml::from_str(&text).unwrap();
    assert_eq!(
        parsed,
        json!({"providers": [{"id": "a", "headers": [{"name": "x", "value": "y"}]}, {"id": "b", "headers": []}]})
    );
}

#[test]
fn an_unusable_document_is_written_afresh() {
    let text = written("this is = = not toml", json!({"value": 1}));
    assert_eq!(toml::from_str::<Value>(&text).unwrap(), json!({"value": 1}));
}
