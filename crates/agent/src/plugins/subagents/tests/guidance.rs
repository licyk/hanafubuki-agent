//! The guidance's Models part: the enabled Providers, the parent's first,
//! each with its known models and their reasoning efforts, bounded (8
//! Providers of 8 models, 8 efforts each, 4 KiB in all) with what is left
//! out said; built when each step is prepared, so a listing recorded or a
//! refusal learned changes it from the next step, and nothing else does;
//! never a token, a header or a base URL; and everything it lists is what
//! `spawn_agent` accepts, named as its refusals name it.

use hanafubuki_agent_config::{AgentModelOverrideSettings, AgentProviderHeaderSettings};
use hanafubuki_agent_kernel::{KnownReasoning, ModelReasoningEffort};

use super::super::models::models_guidance;
use super::models::{list, runs_on, spawned, with_two_providers};
use super::*;
use crate::session_driver::{SpawnModelCatalog, SpawnProvider};
use crate::test_support::{add_test_provider, http_error, FakeListing};

const INTRO: &str = "`spawn_agent`'s `provider`, `model` and `reasoning_effort` choose a sub-agent's Provider, model and reasoning effort; without them it runs its role's, else yours";
const NO_LISTING: &str = "with no listing known (any model id is passed to it as given)";
const LEFT_OUT: &str =
    "- … the rest of this list is left out; what it leaves out is still accepted";
const REASONING_REFUSAL: &str = r#"{"error":{"message":"Unsupported parameter: 'reasoning.effort' is not supported with this model.","type":"invalid_request_error","param":"reasoning.effort","code":"unsupported_parameter"}}"#;
const UNREACHABLE: &str = "http://127.0.0.1:9/v1";
const SECRET: &str = "sk-guidance-secret-token";

fn effort_of(effort: &str) -> ModelReasoningEffort {
    ModelReasoningEffort::parse(effort).unwrap()
}

fn levels(efforts: &[&str], default: Option<&str>) -> KnownReasoning {
    KnownReasoning::Levels {
        efforts: efforts.iter().copied().map(effort_of).collect(),
        default: default.map(effort_of),
    }
}

fn provider(
    id: &str,
    display_name: &str,
    listed: bool,
    models: Vec<(String, KnownReasoning)>,
    more_models: usize,
) -> SpawnProvider {
    SpawnProvider {
        id: id.to_string(),
        display_name: display_name.to_string(),
        listed,
        models,
        more_models,
    }
}

/// The Models part of the guidance `request` carries, if any.
fn models_part(request: &Value) -> Option<String> {
    let guidance = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"][0]["text"].as_str())
        .find(|text| text.contains("<subagents>"))?;
    let start = guidance
        .find(INTRO)
        .expect("the guidance has a Models part");
    let end = guidance.find("\n</subagents>").unwrap();
    Some(guidance[start..end].to_string())
}

fn request(server: &FakeServer, index: usize) -> Value {
    serde_json::from_slice(&server.requests()[index].body).unwrap()
}

/// A Provider the Models part lists: its id, how it is named, and its
/// model ids, each with its efforts (`(default)` dropped).
struct Listed {
    id: String,
    label: String,
    models: Vec<(String, Vec<String>)>,
}

/// Each Provider the Models part `text` lists.
fn listed(text: &str) -> Vec<Listed> {
    let mut providers = Vec::<Listed>::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("- `") {
            providers.push(Listed {
                id: rest.split('`').next().unwrap().to_string(),
                label: line[2..=line.find(')').unwrap()].to_string(),
                models: Vec::new(),
            });
        } else if let Some(rest) = line.strip_prefix("  - `") {
            let (model, after) = rest.split_once('`').unwrap();
            let efforts = after
                .strip_prefix(": reasoning efforts ")
                .map(|efforts| {
                    efforts
                        .split(", ")
                        .map(|effort| effort.trim_end_matches(" (default)").to_string())
                        .collect()
                })
                .unwrap_or_default();
            let provider = providers.last_mut().unwrap();
            provider.models.push((model.to_string(), efforts));
        }
    }
    providers
}

/// Case (b) of the Models part: a Provider with levels, a model that takes
/// none, one of unknown levels, more than 8 levels, and what is left out;
/// a Provider with no listing, and one whose listing names nothing.
#[test]
fn the_models_part_names_providers_models_and_levels_as_the_refusals_do() {
    let efforts = (0..10).map(|index| format!("e{index}")).collect::<Vec<_>>();
    let many = efforts.iter().map(String::as_str).collect::<Vec<_>>();
    let catalog = SpawnModelCatalog {
        provider_id: "provider-1".into(),
        model: "gpt-5.4".into(),
        providers: vec![
            provider(
                "provider-1",
                "Open\nAI",
                true,
                vec![
                    (
                        "gpt-5.4".into(),
                        levels(&["low", "medium", "high", "xhigh"], Some("medium")),
                    ),
                    ("many".into(), levels(&many, Some("e9"))),
                    (
                        "plain".into(),
                        KnownReasoning::Unsupported { learned: true },
                    ),
                    ("open".into(), KnownReasoning::Unknown),
                ],
                212,
            ),
            provider("provider-2", "Local", false, Vec::new(), 0),
            provider("provider-3", "Empty", true, Vec::new(), 0),
        ],
        more_providers: 3,
    };

    let text = models_guidance(&catalog);

    assert_eq!(
        text,
        format!(
            "{INTRO} (`gpt-5.4` of `provider-1`). The enabled Providers (pass the id) and the models known of them:
- `provider-1` (Open AI), yours:
  - `gpt-5.4`: reasoning efforts low, medium (default), high, xhigh
  - `many`: reasoning efforts e0, e1, e2, e3, e4, e5, e6, e7, and 2 more
  - `plain`: takes no reasoning effort
  - `open`
  - … and 212 more
- `provider-2` (Local), {NO_LISTING}
- `provider-3` (Empty), whose listing names no model
- … and 3 more enabled Providers"
        )
    );
}

/// Long ids, many efforts and long CJK names stop the part at 4 KiB, on a
/// whole line, with the line saying the rest is left out; a name is cut at
/// 64 characters, never inside one. A part that fits is whole.
#[test]
fn the_models_part_stops_at_its_byte_bound_on_a_whole_line() {
    let efforts = [
        "minimal", "low", "medium", "high", "xhigh", "max", "ultra", "turbo",
    ];
    let name = "名".repeat(70);
    let providers = (1..=8)
        .map(|index| {
            let models = (0..8)
                .map(|model| {
                    (
                        format!("{model}{}", "m".repeat(127)),
                        levels(&efforts, Some("medium")),
                    )
                })
                .collect();
            provider(&format!("provider-{index}"), &name, true, models, 0)
        })
        .collect::<Vec<_>>();
    let mut catalog = SpawnModelCatalog {
        provider_id: "provider-1".into(),
        model: "0".repeat(128),
        providers,
        more_providers: 0,
    };

    let text = models_guidance(&catalog);

    assert!(text.len() <= 4096, "{}", text.len());
    assert!(text.ends_with(&format!("\n{LEFT_OUT}")), "{text}");
    let cut = format!("- `provider-1` ({}…), yours:", "名".repeat(64));
    assert_eq!(text.lines().nth(1), Some(cut.as_str()));
    let shown = text
        .lines()
        .filter(|line| line.starts_with("  - `"))
        .count();
    assert!((1..64).contains(&shown), "{shown}");
    assert!(text
        .lines()
        .filter(|line| line.starts_with("  - `"))
        .all(|line| line.ends_with("max, ultra, turbo")));

    catalog.providers.truncate(1);
    catalog.providers[0].models.truncate(2);
    let whole = models_guidance(&catalog);
    assert!(!whole.contains(LEFT_OUT));
    assert_eq!(whole.lines().count(), 4);
}

/// Over the session driver: the parent's Provider first, then by name
/// (case aside) and id; at most 8 Providers of 8 models, in the order the
/// parent's, the configured, the policy's, then the listing's, each once;
/// an id `model` could not take as it is left out and counted; a disabled
/// Provider absent; one id on two Providers listed under both. Case (c).
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn the_catalog_is_ordered_bounded_and_says_what_it_leaves_out() {
    let mut listing = (0..7)
        .map(|index| json!({ "id": format!("l-{index:02}") }))
        .collect::<Vec<_>>();
    for id in [
        "shared-model".to_string(),
        "q".repeat(129),
        "a`b".to_string(),
        "x\u{7}y".to_string(),
        "中".repeat(90),
        "y".repeat(200),
    ] {
        listing.push(json!({ "id": id }));
    }
    let first = FakeServer::start(Vec::new()).await;
    let second = FakeServer::with_listings(
        Vec::new(),
        vec![FakeListing::ok(&json!({ "data": listing }).to_string())],
    )
    .await;
    let names = [
        ("provider-3", "alpha"),
        ("provider-4", "Zeta"),
        ("provider-5", "Beta"),
        ("provider-6", "gamma"),
        ("provider-7", "Delta"),
        ("provider-8", "epsilon"),
        ("provider-9", "delta"),
        ("provider-10", "theta"),
        ("provider-11", "Aardvark"),
    ];
    let (service, paths, _) =
        with_two_providers("subagents-guidance-bounds", &first, &second, |config| {
            for (id, name) in names {
                add_test_provider(config, id, name, UNREACHABLE);
            }
            let revision = config.providers().unwrap().revision;
            config
                .mutate_providers(revision, |registry| {
                    registry.providers[1].display_name = "second".into();
                    registry.providers[10].enabled = false;
                    let configured = &mut registry.providers[0].models;
                    for model in std::iter::once("fixture-model".to_string())
                        .chain((0..9).map(|index| format!("p1-m{index}")))
                    {
                        configured.insert(model, AgentModelOverrideSettings::default());
                    }
                    registry.providers[2]
                        .models
                        .insert("shared-model".into(), Default::default());
                    Ok(())
                })
                .unwrap();
        })
        .await;
    list(&service, "provider-2").await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let snapshot = service.snapshot(parent).unwrap();

    let catalog = subagents.driver.spawn_model_catalog(&snapshot).unwrap();
    let text = models_guidance(&catalog);

    let p1 = (0..7)
        .map(|index| format!("\n  - `p1-m{index}`"))
        .collect::<String>();
    let p2 = (0..7)
        .map(|index| format!("\n  - `l-{index:02}`"))
        .collect::<String>();
    let unlisted = |id: &str, name: &str| format!("\n- `{id}` ({name}), {NO_LISTING}");
    assert_eq!(
        text,
        format!(
            "{INTRO} (`fixture-model` of `provider-1`). The enabled Providers (pass the id) and the models known of them:
- `provider-1` (Fixture Provider), yours, {NO_LISTING}:
  - `fixture-model`{p1}
  - … and 2 more
- `provider-3` (alpha), {NO_LISTING}:
  - `shared-model`{}{}{}{}{}
- `provider-2` (second):{p2}
  - `shared-model`
  - … and 5 more
- … and 2 more enabled Providers",
            unlisted("provider-5", "Beta"),
            unlisted("provider-7", "Delta"),
            unlisted("provider-9", "delta"),
            unlisted("provider-8", "epsilon"),
            unlisted("provider-6", "gamma"),
        )
    );
    assert!(!text.contains("Aardvark") && !text.contains("Zeta"));
    finish(service, paths);
}

/// The request carries the Models part; two steps of a turn, and turns
/// with nothing changed, carry it byte for byte; a listing recorded
/// between turns changes it once. No token, header or base URL is in it.
/// Every Provider, model and effort it lists spawns, and the refusals name
/// a Provider and a model's levels as it does. Cases (a) and (b).
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn the_request_lists_what_a_spawn_accepts_and_changes_only_with_a_listing() {
    let first = FakeServer::start(vec![
        (
            tool_calls_events(&[("call-list", "list_agents", "{}")]),
            Duration::ZERO,
        ),
        (assistant_events("Listed."), Duration::ZERO),
        (assistant_events("Again."), Duration::ZERO),
        (assistant_events("Once more."), Duration::ZERO),
        (assistant_events("Done."), Duration::ZERO),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let listing = json!({ "data": [
        {"id": "levels-model",
         "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}],
         "default_reasoning_level": "medium"},
        {"id": "plain-model", "reasoning": false},
        {"id": "open-model"},
        {"id": "fixture-model"},
    ]});
    let second = FakeServer::with_listings(
        vec![(assistant_events("Done."), Duration::ZERO); 7],
        vec![FakeListing::ok(&listing.to_string())],
    )
    .await;
    let (service, paths, _) =
        with_two_providers("subagents-guidance-request", &first, &second, |config| {
            let revision = config.providers().unwrap().revision;
            config
                .mutate_providers(revision, |registry| {
                    let first = &mut registry.providers[0];
                    first.api_token = Some(SECRET.into());
                    first.request_headers.push(AgentProviderHeaderSettings {
                        name: "x-guidance-header".into(),
                        value: "header-value".into(),
                    });
                    first
                        .models
                        .insert("configured-model".into(), Default::default());
                    Ok(())
                })
                .unwrap();
        })
        .await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .set_session_permission_mode(parent.id, AgentPermissionMode::FullAccess)
        .unwrap();
    let parent = parent.id;
    let turn = |text: &'static str| {
        let service = service.clone();
        async move {
            service
                .start_turn(parent, text.to_string().into())
                .await
                .unwrap();
        }
    };

    turn("First.").await;
    let before = models_part(&request(&first, 0)).unwrap();
    assert_eq!(
        before,
        format!(
            "{INTRO} (`fixture-model` of `provider-1`). The enabled Providers (pass the id) and the models known of them:
- `provider-1` (Fixture Provider), yours, {NO_LISTING}:
  - `fixture-model`
  - `configured-model`
- `provider-2` (Second), {NO_LISTING}"
        )
    );
    assert_eq!(models_part(&request(&first, 1)).as_deref(), Some(&*before));
    list(&service, "provider-2").await;
    turn("Second.").await;
    turn("Third.").await;
    let after = models_part(&request(&first, 2)).unwrap();
    assert_eq!(
        after,
        format!(
            "{INTRO} (`fixture-model` of `provider-1`). The enabled Providers (pass the id) and the models known of them:
- `provider-1` (Fixture Provider), yours, {NO_LISTING}:
  - `fixture-model`
  - `configured-model`
- `provider-2` (Second):
  - `fixture-model`
  - `levels-model`: reasoning efforts low, medium (default), high
  - `open-model`
  - `plain-model`: takes no reasoning effort"
        )
    );
    assert_eq!(models_part(&request(&first, 3)).as_deref(), Some(&*after));
    for request in first.requests() {
        let body = String::from_utf8_lossy(&request.body);
        assert!(!body.contains(SECRET) && !body.contains("header-value"));
    }
    for part in [&before, &after] {
        assert!(!part.contains(&first.base_url) && !part.contains(&second.base_url));
        assert!(!part.contains("127.0.0.1") && !part.contains("http"));
    }

    let mut spawns = 0;
    for Listed {
        id: provider_id,
        models,
        ..
    } in listed(&after)
    {
        for (model, efforts) in models {
            let choices = std::iter::once(None).chain(efforts.iter().map(Some));
            for effort in choices {
                let mut arguments = json!({ "provider": provider_id, "model": model });
                if let Some(effort) = effort {
                    arguments["reasoning_effort"] = json!(effort);
                }
                let task_name = format!("agent_{spawns}");
                let result = spawned(&subagents, parent, &task_name, arguments).await;
                assert_eq!(
                    runs_on(&result).0,
                    provider_id,
                    "{task_name}: {provider_id} {model} {effort:?}"
                );
                assert_eq!(runs_on(&result).1, model);
                if let Some(effort) = effort {
                    assert_eq!(runs_on(&result).2, Some(effort.as_str()));
                }
                spawns += 1;
            }
        }
    }
    assert_eq!(spawns, 9);

    let refused = |arguments: Value| {
        let subagents = subagents.clone();
        async move {
            let mut call_arguments = json!({ "task_name": "refused", "message": "Refused." });
            for (key, value) in arguments.as_object().unwrap() {
                call_arguments[key] = value.clone();
            }
            match call(
                &subagents,
                SubagentToolKind::SpawnAgent,
                parent,
                call_arguments,
            )
            .await
            {
                Err(AgentToolError::Failed(error)) => error,
                other => panic!("not refused: {other:?}"),
            }
        }
    };
    let labels = listed(&after)
        .into_iter()
        .map(|provider| provider.label)
        .collect::<Vec<_>>();
    let provider = refused(json!({ "provider": "nope" })).await;
    assert!(
        provider.ends_with(&format!("use one of {}", labels.join(", "))),
        "{provider}"
    );
    let model = refused(json!({ "provider": "provider-2", "model": "nope" })).await;
    assert!(
        model.contains(&format!("is not a model Provider {} lists", labels[1])),
        "{model}"
    );
    let effort = refused(
        json!({ "provider": "provider-2", "model": "levels-model", "reasoning_effort": "xhigh" }),
    )
    .await;
    let shown = after
        .lines()
        .find_map(|line| line.strip_prefix("  - `levels-model`: reasoning efforts "))
        .unwrap();
    assert!(effort.ends_with(&format!("use one of {shown}")), "{effort}");
    let none = refused(
        json!({ "provider": "provider-2", "model": "plain-model", "reasoning_effort": "low" }),
    )
    .await;
    assert!(
        none.contains("`plain-model` takes no reasoning effort"),
        "{none}"
    );
    assert!(after.contains("`plain-model`: takes no reasoning effort"));
    finish(service, paths);
}

/// A refusal of the reasoning parameter that the Provider teaches turns
/// the model's levels into "takes no reasoning effort" from the next step,
/// and forgetting it turns them back. (Its 7-day expiry is the model
/// metadata's, not tested here.)
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_learned_reasoning_refusal_changes_the_models_part_until_it_is_forgotten() {
    let first = FakeServer::with_listings(
        vec![
            (http_error(400, REASONING_REFUSAL), Duration::ZERO),
            (assistant_events("Thought less."), Duration::ZERO),
            (assistant_events("Again."), Duration::ZERO),
            (assistant_events("Once more."), Duration::ZERO),
        ],
        vec![FakeListing::ok(
            r#"{"data":[{"id":"fixture-model","supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"}],"default_reasoning_level":"medium"}]}"#,
        )],
    )
    .await;
    let second = FakeServer::start(Vec::new()).await;
    let (service, paths, _) =
        with_two_providers("subagents-guidance-learned", &first, &second, |_| {}).await;
    list(&service, "provider-1").await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let levels = "  - `fixture-model`: reasoning efforts low, medium (default)";
    let refused = "  - `fixture-model`: takes no reasoning effort";

    service
        .start_turn(parent, "Think.".to_string().into())
        .await
        .unwrap();
    let thought = models_part(&request(&first, 0)).unwrap();
    assert!(thought.contains(levels), "{thought}");
    assert_eq!(request(&first, 0)["reasoning"]["effort"], "medium");
    assert!(request(&first, 1).get("reasoning").is_none());
    service
        .start_turn(parent, "Again.".to_string().into())
        .await
        .unwrap();
    let learned = models_part(&request(&first, 2)).unwrap();
    assert_eq!(learned, thought.replace(levels, refused));
    assert_eq!(
        service
            .forget_learned_model_facts("provider-1", None)
            .await
            .unwrap(),
        1
    );
    service
        .start_turn(parent, "Once more.".to_string().into())
        .await
        .unwrap();
    assert_eq!(models_part(&request(&first, 3)).unwrap(), thought);
    finish(service, paths);
}

/// Neither a sub-agent nor a Plan-mode session is shown the Models part
/// (nor the rest of the guidance).
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_sub_agent_and_a_plan_mode_session_are_not_shown_it() {
    let first = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 2]).await;
    let second = FakeServer::start(Vec::new()).await;
    let (service, paths, _) =
        with_two_providers("subagents-guidance-absent", &first, &second, |_| {}).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    spawned(&subagents, parent, "child", json!({})).await;
    let planner = service
        .create_session(AgentSessionMode::Plan, None)
        .await
        .unwrap()
        .id;
    service
        .start_turn(planner, "Plan it.".to_string().into())
        .await
        .unwrap();

    let child = request_with(&first, "Task child.").await;
    let plan = request_with(&first, "Plan it.").await;
    for request in [child, plan] {
        assert!(models_part(&request).is_none());
        assert!(!request.to_string().contains(INTRO));
    }
    finish(service, paths);
}
