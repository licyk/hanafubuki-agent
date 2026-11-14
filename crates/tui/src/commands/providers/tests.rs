use std::sync::mpsc::{channel, Receiver};

use hanafubuki_agent::{
    AgentListedModel, AgentProviderRegistrySnapshot, ModelCatalogEntry, ModelMetadata,
    ModelMetadataSource, ModelReasoningEffort, ModelReasoningSupport, Sourced,
};
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use serde_json::json;
use unicode_width::UnicodeWidthStr;

use super::models::ModelPicker;
use crate::agent_bridge::{setup_needed, ProviderModels};
use crate::app::{SettingsPageAction, WorkspaceEvent};
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::{BottomPaneView, LoadingLine};
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;

#[test]
fn the_setup_is_needed_without_a_default_provider_or_a_model() {
    let mut registry = AgentProviderRegistrySnapshot::default();
    assert!(setup_needed(&registry, false));
    assert!(setup_needed(&registry, true));
    registry.default_provider_id = Some("provider-1".into());
    assert!(setup_needed(&registry, false));
    // `-m` gives the sessions their model.
    assert!(!setup_needed(&registry, true));
    registry.model_policy.model = "gpt-5.1".into();
    assert!(!setup_needed(&registry, false));
}

const PROVIDER: &str = "anthropic";

fn picker() -> (ModelPicker, Receiver<AppEvent>) {
    let (sender, events) = channel();
    let picker = ModelPicker::new(
        PROVIDER.to_string(),
        AppEventSender::new(sender),
        LoadingLine::still(),
    );
    (picker, events)
}

fn hear(picker: &mut ModelPicker, result: Result<Vec<AgentListedModel>, String>) {
    hear_with(picker, result, false);
}

fn hear_with(
    picker: &mut ModelPicker,
    result: Result<Vec<AgentListedModel>, String>,
    models_dev: bool,
) {
    let left = picker.take_command_reply(CommandReply::ProviderModels(ProviderModels {
        provider_id: PROVIDER.to_string(),
        name: "Anthropic".to_string(),
        current: Some("claude-haiku-4-5".to_string()),
        models_dev,
        result,
    }));
    assert!(left.is_none(), "the page's listing is the page's");
}

fn key(picker: &mut ModelPicker, code: KeyCode) {
    picker.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn render(picker: &ModelPicker, width: u16) -> String {
    let area = Rect::new(0, 0, width, picker.desired_height(width));
    let mut buf = Buffer::empty(area);
    picker.render(area, &mut buf);
    (0..area.height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0;
            // A wide character's second cell is drawn by the first.
            while x < area.width {
                let symbol = buf[(x, y)].symbol();
                line.push_str(symbol);
                x += symbol.width().max(1) as u16;
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn at_widths(picker: &ModelPicker) -> String {
    [100, 80, 50]
        .map(|width| format!("--- {width} columns ---\n{}", render(picker, width)))
        .join("\n")
}

fn fact<T>(value: T, source: ModelMetadataSource) -> Option<Sourced<T>> {
    Some(Sourced {
        value,
        source,
        observed_at_ms: 0,
    })
}

/// A model `id`, named `display_name` by its listing, with a window of
/// `window` (`0`: not known), whether it thinks and reads images, all as
/// its listing reports.
fn model(
    id: &str,
    display_name: Option<&str>,
    window: u64,
    thinks: Option<bool>,
    vision: Option<bool>,
) -> AgentListedModel {
    let reported = ModelMetadataSource::Reported;
    AgentListedModel {
        entry: ModelCatalogEntry {
            id: id.to_string(),
            display_name: display_name.map(str::to_string),
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            context_window: (window > 0).then_some(window),
            max_output_tokens: None,
            image_input: vision,
            reasoning: thinks,
        },
        metadata: ModelMetadata {
            context_window: (window > 0)
                .then_some(window)
                .and_then(|window| fact(window, reported)),
            reasoning: thinks.and_then(|thinks| {
                let support = if thinks {
                    ModelReasoningSupport::Supported {
                        efforts: Vec::new(),
                        default: None,
                    }
                } else {
                    ModelReasoningSupport::Unsupported
                };
                fact(support, reported)
            }),
            image_input: vision.and_then(|vision| fact(vision, reported)),
            ..ModelMetadata::default()
        },
    }
}

/// Anthropic's listing: the default model second, with its levels and
/// largest output; one model reporting nothing.
fn anthropic() -> Vec<AgentListedModel> {
    let mut haiku = model(
        "claude-haiku-4-5",
        Some("Claude Haiku 4.5"),
        200_000,
        Some(true),
        Some(true),
    );
    haiku.metadata.max_output_tokens = fact(64_000, ModelMetadataSource::Reported);
    haiku.metadata.reasoning = fact(
        ModelReasoningSupport::Supported {
            efforts: vec![ModelReasoningEffort::Low, ModelReasoningEffort::High],
            default: None,
        },
        ModelMetadataSource::Reported,
    );
    vec![
        model(
            "claude-sonnet-4-5",
            Some("Claude Sonnet 4.5"),
            200_000,
            Some(true),
            Some(true),
        ),
        haiku,
        model("claude-2.1", None, 0, Some(false), None),
    ]
}

fn chosen(events: &Receiver<AppEvent>) -> Vec<SettingsPageAction> {
    std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            AppEvent::Workspace(WorkspaceEvent::SettingsAction(action)) => Some(action),
            _ => None,
        })
        .collect()
}

#[test]
fn while_the_models_are_listed_the_page_shows_the_loading_line() {
    let (picker, _events) = picker();
    let rendered = at_widths(&picker);
    assert!(rendered.contains("• Listing the models…"), "{rendered}");
    assert_snapshot!("provider_models_listing", rendered);
}

#[test]
fn the_models_show_their_facts_in_columns_and_the_selected_one_s_under_the_list() {
    let (mut picker, _events) = picker();
    hear(&mut picker, Ok(anthropic()));
    let rendered = at_widths(&picker);
    assert!(!rendered.contains("Listing the models…"), "{rendered}");
    assert_snapshot!("provider_models_listed", rendered);

    // The entry row has no facts to tell.
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Down);
    let entry = render(&picker, 80);
    assert!(!entry.contains("Anthropic: context"), "{entry}");
    assert_snapshot!("provider_models_entry_row_selected", entry);
}

#[test]
fn models_dev_is_named_only_while_it_is_off_and_a_fact_is_not_known() {
    let (mut picker, _events) = picker();
    hear_with(&mut picker, Ok(anthropic()), true);
    let screen: String = render(&picker, 100).split_whitespace().collect();
    assert!(screen.contains("enterchoose·escback·?notknown"), "{screen}");
    assert!(!screen.contains("models.dev"), "{screen}");

    let (mut picker, _events) = self::picker();
    hear(&mut picker, Ok(anthropic()[..1].to_vec()));
    let screen = render(&picker, 100);
    assert!(
        !screen.contains("models.dev"),
        "every fact known:\n{screen}"
    );
}

#[test]
fn a_failed_listing_leaves_the_entry_row_and_says_why() {
    let (mut picker, _events) = picker();
    hear(&mut picker, Err("401 Unauthorized\nbad token".to_string()));
    assert_snapshot!("provider_models_failed", at_widths(&picker));
}

#[test]
fn wide_characters_and_ids_longer_than_the_screen_are_cut_in_their_rows() {
    let (mut picker, _events) = picker();
    let long = format!("org/{}", "very-long-model-name-".repeat(6));
    hear(
        &mut picker,
        Ok(vec![
            model(
                "通义千问-编程-最新版本",
                Some("通义千问 编程版"),
                32_768,
                Some(true),
                None,
            ),
            model(&long, None, 2_000_000, None, Some(true)),
        ]),
    );
    key(&mut picker, KeyCode::Down);
    let rendered = at_widths(&picker);
    for width in [100, 80, 50] {
        let unwrapped: String = render(&picker, width).split_whitespace().collect();
        assert!(
            unwrapped.contains(&format!("{long}·Anthropic:context2M(reported)")),
            "the detail line has it whole at {width}:\n{rendered}"
        );
    }
    assert_snapshot!("provider_models_wide_and_long", rendered);
}

#[test]
fn a_model_or_an_id_typed_in_is_saved_as_the_policy_s_with_the_provider() {
    let (mut picker, events) = picker();
    hear(&mut picker, Ok(anthropic()));
    // The default model starts selected.
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Enter);
    let saved = chosen(&events);
    let [action] = saved.as_slice() else {
        panic!("{saved:?}");
    };
    assert_eq!(
        (action.collection.as_str(), action.record.as_deref()),
        ("modelPolicy", Some("policy"))
    );
    assert_eq!(action.action, "save");
    assert_eq!(
        serde_json::Value::Object(action.values.clone()),
        json!({"defaultProviderId": PROVIDER, "model": "claude-2.1"})
    );
    assert!(picker.is_complete());

    let (mut picker, events) = self::picker();
    hear(&mut picker, Ok(anthropic()));
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Enter);
    assert!(render(&picker, 80).contains("Enter a model id"));
    picker.handle_paste("claude-opus-5".to_string());
    // The form's field, then its action.
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Enter);
    let saved = chosen(&events);
    assert_eq!(
        saved
            .iter()
            .map(|action| action.values["model"].clone())
            .collect::<Vec<_>>(),
        vec![json!("claude-opus-5")]
    );
}

#[test]
fn another_provider_s_listing_is_not_the_page_s() {
    let (mut picker, _events) = picker();
    let other = CommandReply::ProviderModels(ProviderModels {
        provider_id: "openai".to_string(),
        name: "OpenAI".to_string(),
        current: None,
        models_dev: false,
        result: Ok(Vec::new()),
    });
    assert!(picker.take_command_reply(other).is_some());
    assert!(render(&picker, 80).contains("Listing the models…"));
    key(&mut picker, KeyCode::Esc);
    assert!(picker.is_complete());
}
