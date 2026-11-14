use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use hanafubuki_agent::{
    AgentListedModel, AgentProviderModelCatalogResult, ModelCatalogEntry, ModelMetadata,
    ModelMetadataSource, ModelReasoningEffort, ModelReasoningSupport, Sourced,
};
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use super::view::ModelListView;
use crate::agent_bridge::model_list::{
    CurrentModel, ModelListEvent, ModelListStart, ModelListUpdate,
};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{BottomPaneView, LoadingLine};
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;

const LISTING: u64 = 7;

fn view() -> (ModelListView, Receiver<AppEvent>) {
    let (sender, events) = channel();
    let view = ModelListView::new(LISTING, AppEventSender::new(sender), LoadingLine::still());
    (view, events)
}

fn hear(view: &mut ModelListView, event: ModelListEvent) {
    hear_as(view, LISTING, event);
}

fn hear_as(view: &mut ModelListView, listing: u64, event: ModelListEvent) {
    let left = view.take_command_reply(CommandReply::ModelList(ModelListUpdate { listing, event }));
    assert!(left.is_none(), "a list's answer is the list's");
}

fn key(view: &mut ModelListView, code: KeyCode) {
    view.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn typed(view: &mut ModelListView, text: &str) {
    text.chars().for_each(|c| key(view, KeyCode::Char(c)));
}

fn render(view: &ModelListView, width: u16) -> String {
    let area = Rect::new(0, 0, width, view.desired_height(width));
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);
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

/// The view at the widths the plan drew it at.
fn at_widths(view: &ModelListView) -> String {
    [100, 80, 50]
        .map(|width| format!("--- {width} columns ---\n{}", render(view, width)))
        .join("\n")
}

fn sent(events: &Receiver<AppEvent>) -> Vec<AgentCommand> {
    std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            AppEvent::Op(Op::Command(command)) => Some(command),
            _ => None,
        })
        .collect()
}

fn fact<T>(value: T, source: ModelMetadataSource) -> Option<Sourced<T>> {
    Some(Sourced {
        value,
        source,
        observed_at_ms: 0,
    })
}

/// A model `id` with a window of `window` (`0`: not known), whether it
/// thinks and reads images, all as its listing reports.
fn model(id: &str, window: u64, thinks: Option<bool>, vision: Option<bool>) -> AgentListedModel {
    let reported = ModelMetadataSource::Reported;
    AgentListedModel {
        entry: ModelCatalogEntry {
            id: id.to_string(),
            display_name: None,
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
                .and_then(|w| fact(w, reported)),
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

/// The session's model, as `model_metadata` knows it before any listing.
fn sonnet_facts() -> ModelMetadata {
    let reported = ModelMetadataSource::Reported;
    ModelMetadata {
        context_window: fact(200_000, reported),
        max_output_tokens: fact(64_000, reported),
        reasoning: fact(
            ModelReasoningSupport::Supported {
                efforts: vec![
                    ModelReasoningEffort::Low,
                    ModelReasoningEffort::Medium,
                    ModelReasoningEffort::High,
                ],
                default: None,
            },
            reported,
        ),
        image_input: fact(true, reported),
        ..ModelMetadata::default()
    }
}

fn start(providers: &[(&str, &str)]) -> ModelListEvent {
    ModelListEvent::Started(Box::new(ModelListStart {
        providers: providers
            .iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect(),
        current: Some(CurrentModel {
            provider_id: "anthropic".to_string(),
            provider: "Anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            metadata: sonnet_facts(),
        }),
        policy_note: None,
        models_dev: false,
    }))
}

const PROVIDERS: [(&str, &str); 6] = [
    ("anthropic", "Anthropic"),
    ("local", "Local"),
    ("openai", "OpenAI"),
    ("openrouter", "OpenRouter"),
    ("team", "Team gateway"),
    ("work", "Work proxy"),
];

fn listed(provider_id: &str, models: Vec<AgentListedModel>) -> ModelListEvent {
    ModelListEvent::Listed(AgentProviderModelCatalogResult {
        provider_id: provider_id.to_string(),
        display_name: provider_id.to_string(),
        models,
        error: None,
    })
}

fn failed(provider_id: &str, error: &str) -> ModelListEvent {
    ModelListEvent::Listed(AgentProviderModelCatalogResult {
        provider_id: provider_id.to_string(),
        display_name: provider_id.to_string(),
        models: Vec::new(),
        error: Some(error.to_string()),
    })
}

fn anthropic() -> ModelListEvent {
    let mut sonnet = model("claude-sonnet-4-5", 200_000, Some(true), Some(true));
    sonnet.metadata = sonnet_facts();
    listed(
        "anthropic",
        vec![
            sonnet,
            model("claude-haiku-4-5", 200_000, Some(true), Some(true)),
        ],
    )
}

fn openai() -> ModelListEvent {
    let built_in = |id: &str, window| {
        let mut listed = model(id, 0, None, None);
        listed.metadata.context_window = fact(window, ModelMetadataSource::BuiltIn);
        listed
    };
    listed(
        "openai",
        vec![built_in("gpt-4.1", 1_047_576), built_in("gpt-5.1", 272_000)],
    )
}

fn openrouter() -> ModelListEvent {
    listed(
        "openrouter",
        vec![model(
            "meta-llama/llama-3.1-405b-instruct:free",
            131_072,
            Some(false),
            Some(false),
        )],
    )
}

fn local() -> ModelListEvent {
    listed("local", vec![model("qwen3-coder:30b", 0, None, None)])
}

#[test]
fn the_list_opens_before_anything_is_heard() {
    let (view, _events) = view();
    assert_snapshot!("model_list_opened", at_widths(&view));
}

#[test]
fn started_the_session_s_model_leads_and_each_provider_is_listing() {
    let (mut view, _events) = view();
    hear(&mut view, start(&PROVIDERS));
    assert_snapshot!("model_list_started", at_widths(&view));
}

#[test]
fn each_provider_fills_its_section_as_it_answers() {
    let (mut view, _events) = view();
    hear(&mut view, start(&PROVIDERS));
    hear(&mut view, openai());
    hear(&mut view, failed("work", "401 Unauthorized"));
    hear(&mut view, anthropic());
    assert_snapshot!("model_list_partly_answered", at_widths(&view));
}

#[test]
fn all_answered_the_loading_line_goes_and_a_failed_provider_stays_a_row() {
    let (mut view, _events) = view();
    hear(&mut view, start(&PROVIDERS));
    for event in [
        anthropic(),
        local(),
        openai(),
        openrouter(),
        listed(
            "team",
            vec![model("team-coder", 128_000, None, Some(false))],
        ),
        failed("work", "401 Unauthorized"),
    ] {
        hear(&mut view, event);
    }
    let rendered = at_widths(&view);
    assert!(!rendered.contains("Listing models…"), "{rendered}");
    assert_snapshot!("model_list_all_answered", rendered);
    // The failed row, selected, says it all and what Enter does.
    for _ in 0..7 {
        key(&mut view, KeyCode::Down);
    }
    assert_snapshot!("model_list_failed_row_selected", render(&view, 80));
}

#[test]
fn wide_characters_and_ids_longer_than_the_screen_are_cut_in_their_rows() {
    let (mut view, _events) = view();
    let long = format!("org/{}", "very-long-model-name-".repeat(6));
    hear(
        &mut view,
        start(&[("anthropic", "Anthropic"), ("local", "本地模型")]),
    );
    hear(
        &mut view,
        listed(
            "local",
            vec![
                model("通义千问-编程-最新版本", 32_768, Some(true), None),
                model(&long, 2_000_000, None, Some(true)),
            ],
        ),
    );
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    let rendered = at_widths(&view);
    for width in [100, 80, 50] {
        let unwrapped: String = render(&view, width).split_whitespace().collect();
        assert!(
            unwrapped.contains(&format!("{long}·本地模型:context2M(reported)")),
            "the detail line has it whole at {width}:\n{rendered}"
        );
    }
    assert_snapshot!("model_list_wide_and_long", rendered);
}

#[test]
fn the_policy_s_overrides_are_said_once_and_models_dev_named_while_off() {
    let (mut view, _events) = view();
    let ModelListEvent::Started(mut started) = start(&[("openai", "OpenAI")]) else {
        unreachable!()
    };
    started.policy_note = Some("The model policy never sends images.".to_string());
    hear(&mut view, ModelListEvent::Started(started));
    hear(&mut view, openai());
    let screen = render(&view, 100);
    assert!(
        screen.contains("Switch the model of this session. The model policy never sends images."),
        "{screen}"
    );
    assert!(
        screen.contains("models.dev can fill these: /config modelMetadata"),
        "{screen}"
    );
    assert_snapshot!("model_list_policy_and_models_dev_hint", at_widths(&view));

    // On, or nothing unknown shown: no hint.
    let (mut view, _events) = super::tests::view();
    let ModelListEvent::Started(mut started) = start(&[("openai", "OpenAI")]) else {
        unreachable!()
    };
    started.models_dev = true;
    hear(&mut view, ModelListEvent::Started(started));
    hear(&mut view, openai());
    assert!(!render(&view, 100).contains("models.dev can fill"));
    let (mut view, _events) = super::tests::view();
    hear(&mut view, start(&[("anthropic", "Anthropic")]));
    hear(&mut view, anthropic());
    assert!(!render(&view, 100).contains("models.dev can fill"));
}

#[test]
fn no_provider_enabled_and_providers_unread_say_so() {
    let (mut view, _events) = view();
    hear(&mut view, start(&[]));
    let screen = render(&view, 80);
    assert!(
        screen.contains("No Provider is enabled: /providers adds one."),
        "{screen}"
    );
    assert!(screen.contains("› claude-sonnet-4-5 (current)"), "{screen}");

    let (mut view, _events) = super::tests::view();
    hear(
        &mut view,
        ModelListEvent::Unread("providers.toml is unreadable".to_string()),
    );
    assert_snapshot!("model_list_unread", render(&view, 80));
    // A late start for it changes nothing.
    hear(&mut view, start(&PROVIDERS));
    assert!(render(&view, 80).contains("The Providers were not read"));
}

#[test]
fn another_lists_answers_are_dropped() {
    let (mut view, _events) = view();
    hear_as(&mut view, LISTING - 1, start(&PROVIDERS));
    assert!(render(&view, 80).contains("Listing models…"));
    assert!(!render(&view, 80).contains("claude-sonnet-4-5"));
    hear(&mut view, start(&PROVIDERS));
    hear_as(&mut view, LISTING + 1, anthropic());
    assert!(!render(&view, 80).contains("claude-haiku-4-5"));
    hear(&mut view, anthropic());
    assert!(render(&view, 80).contains("claude-haiku-4-5"));
}

#[test]
fn the_session_s_model_is_shown_once_and_takes_its_listing_s_facts() {
    let (mut view, _events) = view();
    let ModelListEvent::Started(mut started) = start(&[("anthropic", "Anthropic")]) else {
        unreachable!()
    };
    // Before its listing: what `model_metadata` knew (nothing here).
    started.current.as_mut().unwrap().metadata = ModelMetadata::default();
    hear(&mut view, ModelListEvent::Started(started));
    let screen = render(&view, 100);
    assert!(screen.contains("› claude-sonnet-4-5 (current)"), "{screen}");
    assert!(screen.contains("context not known"), "{screen}");
    hear(&mut view, anthropic());
    let screen = render(&view, 100);
    assert_eq!(
        screen.matches("claude-sonnet-4-5").count(),
        2,
        "row and detail:\n{screen}"
    );
    assert!(screen.contains("context 200K (reported)"), "{screen}");
}

/// Its row's text, and the screen line it is on.
fn selected_line(screen: &str) -> (usize, String) {
    screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.starts_with('›'))
        .map(|(at, line)| (at, line.to_string()))
        .expect("a row is selected")
}

#[test]
fn a_section_arriving_above_keeps_the_selected_row_its_line_and_the_search() {
    let (mut view, _events) = view();
    hear(
        &mut view,
        start(&[("first", "First"), ("second", "Second")]),
    );
    let models = |prefix: &str, count: usize| {
        (0..count)
            .map(|n| model(&format!("{prefix}-model-{n:02}"), 128_000, None, None))
            .collect::<Vec<_>>()
    };
    hear(&mut view, listed("second", models("second", 20)));
    // Scrolled, the selected row mid-list.
    for _ in 0..11 {
        key(&mut view, KeyCode::Down);
    }
    for _ in 0..3 {
        key(&mut view, KeyCode::Up);
    }
    let (line, row) = selected_line(&render(&view, 80));
    assert!(row.contains("second-model-07"), "{row}");
    assert!(!render(&view, 80).contains("second-model-00"), "scrolled");
    hear(&mut view, listed("first", models("first", 5)));
    assert_eq!(selected_line(&render(&view, 80)), (line, row));

    // A search, kept as the other rows change.
    typed(&mut view, "model-1");
    let (_, row) = selected_line(&render(&view, 80));
    hear(&mut view, failed("first", "gone"));
    let screen = render(&view, 80);
    assert!(screen.contains("model-1"), "{screen}");
    assert_eq!(selected_line(&screen).1, row);
}

#[test]
fn enter_on_a_failed_provider_lists_it_again_and_on_a_model_sets_it() {
    let (mut view, events) = view();
    hear(
        &mut view,
        start(&[("anthropic", "Anthropic"), ("work", "Work proxy")]),
    );
    hear(&mut view, anthropic());
    hear(&mut view, failed("work", "401 Unauthorized"));
    assert!(!render(&view, 80).contains("Listing models…"));
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    let screen = render(&view, 80);
    assert!(screen.contains("enter lists again · esc close"), "{screen}");
    key(&mut view, KeyCode::Enter);
    assert!(!view.is_complete());
    assert_eq!(
        sent(&events),
        vec![AgentCommand::ListModels {
            listing: LISTING,
            provider_ids: Some(vec!["work".to_string()]),
        }]
    );
    let screen = render(&view, 80);
    assert!(
        screen.contains("Listing models… 1 of 2 Providers answered"),
        "{screen}"
    );
    assert!(screen.contains("listing…"), "{screen}");
    hear(
        &mut view,
        listed("work", vec![model("work-model", 64_000, None, None)]),
    );
    let (_, row) = selected_line(&render(&view, 80));
    assert!(
        row.contains("work-model"),
        "the cursor lands on its models: {row}"
    );
    key(&mut view, KeyCode::Enter);
    assert!(view.is_complete());
    assert_eq!(
        sent(&events),
        vec![AgentCommand::SetModel {
            provider_id: "work".to_string(),
            model: "work-model".to_string(),
        }]
    );
}

#[test]
fn search_matches_the_id_the_display_name_and_the_provider() {
    let (mut view, _events) = view();
    hear(
        &mut view,
        start(&[("openrouter", "OpenRouter"), ("openai", "OpenAI")]),
    );
    let mut named = model("anthropic/claude-sonnet-4.5", 1_000_000, None, None);
    named.entry.display_name = Some("Anthropic: Claude Sonnet 4.5".to_string());
    hear(&mut view, listed("openrouter", vec![named]));
    hear(&mut view, openai());
    typed(&mut view, "Sonnet 4.5");
    let screen = render(&view, 100);
    assert!(screen.contains("anthropic/claude-sonnet-4.5"), "{screen}");
    assert!(!screen.contains("gpt-5.1"), "{screen}");
    for _ in 0.."Sonnet 4.5".len() {
        key(&mut view, KeyCode::Backspace);
    }
    typed(&mut view, "openai");
    let screen = render(&view, 100);
    assert!(
        screen.contains("gpt-4.1") && screen.contains("gpt-5.1"),
        "{screen}"
    );
    assert!(!screen.contains("anthropic/claude"), "{screen}");
}

#[test]
fn the_same_id_on_two_providers_is_two_rows_one_of_them_current() {
    let (mut view, _events) = view();
    hear(
        &mut view,
        start(&[("anthropic", "Anthropic"), ("proxy", "Work proxy")]),
    );
    hear(&mut view, anthropic());
    hear(
        &mut view,
        listed(
            "proxy",
            vec![model("claude-sonnet-4-5", 128_000, None, None)],
        ),
    );
    let screen = render(&view, 100);
    let rows: Vec<&str> = screen
        .lines()
        .filter(|line| line.contains("claude-sonnet-4-5") && !line.contains(" · "))
        .collect();
    assert_eq!(rows.len(), 2, "{screen}");
    assert!(
        rows[0].contains("(current)") && rows[0].contains("Anthropic"),
        "{screen}"
    );
    assert!(
        !rows[1].contains("(current)") && rows[1].contains("Work proxy"),
        "{screen}"
    );
    assert!(rows[1].contains("128K"), "its own route's facts:\n{screen}");
}

#[test]
fn the_session_s_model_missing_from_its_listing_keeps_what_was_known() {
    let (mut view, _events) = view();
    hear(&mut view, start(&[("anthropic", "Anthropic")]));
    hear(
        &mut view,
        listed(
            "anthropic",
            vec![model("claude-haiku-4-5", 200_000, None, None)],
        ),
    );
    let screen = render(&view, 100);
    assert!(
        screen.contains("claude-sonnet-4-5 (current)"),
        "pinned still:\n{screen}"
    );
    let unwrapped: String = screen.split_whitespace().collect();
    assert!(
        unwrapped.contains("thinkinglow,medium,high(reported)"),
        "its facts from the start:\n{screen}"
    );
}

#[test]
fn every_provider_failing_ends_the_wait_and_each_is_listed_again() {
    let (mut view, events) = view();
    hear(&mut view, start(&[("a", "First"), ("b", "Second")]));
    hear(&mut view, failed("a", "refused"));
    hear(&mut view, failed("b", "timed out"));
    let screen = render(&view, 80);
    assert!(!screen.contains("Listing models…"), "{screen}");
    assert!(
        screen.contains("not listed: refused") && screen.contains("not listed: timed out"),
        "{screen}"
    );
    for provider in ["a", "b"] {
        key(&mut view, KeyCode::Down);
        key(&mut view, KeyCode::Enter);
        assert_eq!(
            sent(&events),
            vec![AgentCommand::ListModels {
                listing: LISTING,
                provider_ids: Some(vec![provider.to_string()]),
            }]
        );
        // Pending again: the cursor stays on it until it answers.
        hear(&mut view, failed(provider, "again"));
    }
}

#[test]
fn esc_closes_before_and_after_the_start() {
    let (mut view, _events) = view();
    key(&mut view, KeyCode::Esc);
    assert!(view.is_complete());
    let (mut view, events) = super::tests::view();
    hear(&mut view, start(&PROVIDERS));
    key(&mut view, KeyCode::Esc);
    assert!(view.is_complete());
    assert!(sent(&events).is_empty());
}

/// The plan's bound: 32 Providers of 1 000 models each. Arriving, drawing
/// and searching stay well under a second each here; the bound is for slow
/// runners.
#[test]
fn thousands_of_models_arrive_draw_and_search_in_time() {
    let (mut view, _events) = view();
    let providers: Vec<(String, String)> = (0..32)
        .map(|n| (format!("p{n:02}"), format!("Provider {n:02}")))
        .collect();
    let ModelListEvent::Started(mut started) = start(&[]) else {
        unreachable!()
    };
    started.providers = providers.clone();
    hear(&mut view, ModelListEvent::Started(started));
    let begun = Instant::now();
    for (provider_id, _) in &providers {
        let models = (0..1000)
            .map(|n| {
                model(
                    &format!("{provider_id}-model-{n:04}"),
                    128_000,
                    Some(true),
                    None,
                )
            })
            .collect();
        hear(&mut view, listed(provider_id, models));
    }
    let arrived = begun.elapsed();
    let begun = Instant::now();
    for _ in 0..10 {
        render(&view, 100);
    }
    let drawn = begun.elapsed() / 10;
    let begun = Instant::now();
    typed(&mut view, "p31-model-0999");
    let searched = begun.elapsed();
    let screen = render(&view, 100);
    assert!(screen.contains("p31-model-0999"), "{screen}");
    eprintln!("32 000 models: arrived in {arrived:?}, a frame {drawn:?}, a search {searched:?}");
    let bound = Duration::from_secs(20);
    assert!(arrived < bound && drawn < bound && searched < bound);
}
