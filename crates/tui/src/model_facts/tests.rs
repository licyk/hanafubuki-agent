use hanafubuki_agent::{ModelCatalogEntry, ModelMetadataSource, Sourced};

use super::{entry_badges, sourced_window};

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

#[test]
fn a_window_names_its_source() {
    let window = Sourced {
        value: 1_048_576,
        source: ModelMetadataSource::Learned,
        observed_at_ms: 0,
    };
    assert_eq!(sourced_window(&window), "1.05M, learned");
}

#[test]
fn models_dev_is_named_as_such() {
    let window = Sourced {
        value: 128_000,
        source: ModelMetadataSource::Catalog,
        observed_at_ms: 0,
    };
    assert_eq!(sourced_window(&window), "128K, models.dev");
}
