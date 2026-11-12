use hanafubuki_agent::ModelCatalogEntry;

use super::entry_badges;

fn entry() -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: "model".to_string(),
        display_name: None,
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        context_window: None,
        max_output_tokens: None,
        image_input: None,
        reasoning: None,
    }
}

#[test]
fn an_entry_shows_only_what_it_reports() {
    assert_eq!(entry_badges(&entry()), None);
    let reported = ModelCatalogEntry {
        context_window: Some(200_000),
        image_input: Some(true),
        reasoning: Some(true),
        ..entry()
    };
    assert_eq!(
        entry_badges(&reported).as_deref(),
        Some("200K · vision · reasoning")
    );
    let says_no = ModelCatalogEntry {
        image_input: Some(false),
        reasoning: Some(false),
        ..entry()
    };
    assert_eq!(entry_badges(&says_no), None);
}
