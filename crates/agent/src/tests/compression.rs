//! The `hierarchical` strategy through the facade: switching away from it
//! between turns, editing an earlier message of a folded session, and the
//! state's chain in the history across restarts. The helpers here serve
//! `compression_golden.rs` too.

use std::path::{Path, PathBuf};

use super::*;
use crate::test_support::test_host;
use crate::{AgentCompactionSettings, AgentCompactionStrategy, AgentConfig};
use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_kernel::AgentCompressionState;
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::Value;

const SUMMARY: &str = "The user asked and the assistant answered at length; nothing in it is still open, and no file was changed.";

/// A session switched to `standard` keeps projecting its blocks: the
/// request carries the summary and the `compress` call that holds it, but
/// not what it folded, and no prompt, tool or nudge of the strategy. The
/// next standard compaction ends the blocks.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn switching_to_standard_keeps_the_blocks_until_a_compaction() {
    let first = "The first answer is long. ".repeat(1_000);
    let server = FakeServer::start(
        [
            with_usage(assistant_events(&first), 1_000),
            with_usage(
                call_events(
                    "fold-1",
                    "compress",
                    &json!({ "content": format!("m00002–m00002 First answer\n{SUMMARY}") }),
                ),
                1_000,
            ),
            with_usage(assistant_events("Folded."), 1_000),
            with_usage(assistant_events("Standard now."), 1_000),
            assistant_events("The user asked twice; the first answer was folded."),
            with_usage(assistant_events("Compacted."), 1_000),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths) = service_with("compression-switch-away", |_, config| {
        configure(config, &server.base_url)
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    for prompt in ["Hello.", "Fold it."] {
        service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
    }
    service
        .write_settings(AgentSettingsPatch {
            compaction: Some(AgentCompactionSettings {
                strategy: AgentCompactionStrategy::Standard,
                ..tuned()
            }),
            ..AgentSettingsPatch::default()
        })
        .await
        .unwrap();
    service
        .start_turn(session.id, "Are you there?".to_string().into())
        .await
        .unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    service
        .start_turn(session.id, "And now?".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 6);
    let body = |index: usize| -> Value { serde_json::from_slice(&requests[index].body).unwrap() };
    let carried = body(3);
    let input = carried["input"].as_array().unwrap();
    assert!(!carried["instructions"]
        .as_str()
        .unwrap()
        .contains("ACP TAGS"));
    assert!(carried["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|tool| tool["name"] != "compress" && tool["name"] != "decompress"));
    assert!(input
        .iter()
        .any(|item| item["type"] == "function_call" && item["name"] == "compress"));
    assert!(carried.to_string().contains("</acp>"));
    assert!(!carried.to_string().contains(&first));
    assert!(input.last().unwrap().to_string().contains("Are you there?"));

    let compaction = body(4).to_string();
    assert!(!compaction.contains(&first));
    assert!(compaction.contains("First answer"));
    let after = body(5).to_string();
    assert!(!after.contains("</acp>"));
    assert!(!after.contains(&first));
    finish(service, paths);
}

/// Editing a message after a fold forks the session before that message's
/// turn with the block, which keeps projecting the fork. The fork's chain
/// starts with a snapshot of its own, so a restart reads its state back.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn editing_a_later_message_keeps_the_fold() {
    let first = "The first answer is long. ".repeat(1_000);
    let server = FakeServer::start(
        [
            with_usage(assistant_events(&first), 1_000),
            with_usage(
                call_events(
                    "fold-1",
                    "compress",
                    &json!({ "content": format!("m00002–m00002 First answer\n{SUMMARY}") }),
                ),
                1_000,
            ),
            with_usage(assistant_events("Folded."), 1_000),
            with_usage(assistant_events("Later."), 1_000),
            with_usage(assistant_events("Edited."), 1_000),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths, config) = service_with_config("compression-edit", |_, config| {
        configure(config, &server.base_url)
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    for prompt in ["Hello.", "Fold it.", "Later question."] {
        service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
    }
    let last_turn = service.snapshot(session.id).unwrap().turn.unwrap().id;

    let edited = service.edit_from_turn(session.id, last_turn).unwrap();
    assert_eq!(edited.restored_prompt, "Later question.");
    service
        .start_turn(edited.session.id, "Edited question.".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 5);
    let fork: Value = serde_json::from_slice(&requests[4].body).unwrap();
    let text = fork.to_string();
    assert!(!text.contains(&first));
    assert!(text.contains("First answer"));
    assert!(fork["instructions"].as_str().unwrap().contains("ACP TAGS"));
    let live = compression_state(&service, edited.session.id).unwrap();
    assert_eq!(live.active_blocks().count(), 1);
    shut_down(service);

    let reopened = reopen(config, &paths);
    reopened.core.activate_session(edited.session.id).unwrap();
    assert_eq!(
        compression_state(&reopened, edited.session.id).as_ref(),
        Some(&live)
    );
    finish(reopened, paths);
}

/// A folded session written over many steps: its rollout holds the state
/// as snapshots and the deltas after them, a restart reads back the exact
/// state, so does one that rebuilds the index from the rollouts, and with a
/// link missing from the index the state is rebuilt from the `compress`
/// calls.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn the_state_is_written_as_a_chain_and_read_back_whole() {
    let first = "The first answer is long. ".repeat(1_000);
    let mut responses = vec![
        with_usage(assistant_events(&first), 1_000),
        with_usage(
            call_events(
                "fold-1",
                "compress",
                &json!({ "content": format!("m00002–m00002 First answer\n{SUMMARY}") }),
            ),
            1_000,
        ),
        with_usage(assistant_events("Folded."), 1_000),
    ];
    for index in 0..12 {
        responses.push(with_usage(
            assistant_events(&format!("Answer {index}.")),
            1_000,
        ));
    }
    let server = FakeServer::start(
        responses
            .into_iter()
            .map(|response| (response, Duration::ZERO))
            .collect(),
    )
    .await;
    let (service, paths, config) = service_with_config("compression-chain", |_, config| {
        configure(config, &server.base_url)
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let prompts = ["Hello.".to_string(), "Fold it.".to_string()]
        .into_iter()
        .chain((0..12).map(|index| format!("Question {index}.")));
    for prompt in prompts {
        service.start_turn(session.id, prompt.into()).await.unwrap();
    }
    let live = compression_state(&service, session.id).unwrap();
    assert_eq!(live.active_blocks().count(), 1);
    let rollout = rollouts(&paths);
    let snapshots = rollout
        .matches(r#""compressionUpdate":{"snapshot""#)
        .count();
    let deltas = rollout.matches(r#""compressionUpdate":{"delta""#).count();
    assert!(
        snapshots >= 1 && deltas > snapshots,
        "{snapshots} snapshots, {deltas} deltas"
    );
    shut_down(service);

    let reopened = reopen(config.clone(), &paths);
    reopened.core.activate_session(session.id).unwrap();
    assert_eq!(
        compression_state(&reopened, session.id).as_ref(),
        Some(&live)
    );
    shut_down(reopened);

    // An index of the schema before the chain is rebuilt from the rollouts.
    let index = rusqlite::Connection::open(index_path(&paths)).unwrap();
    index.pragma_update(None, "user_version", 5).unwrap();
    drop(index);
    let reindexed = reopen(config.clone(), &paths);
    reindexed.core.activate_session(session.id).unwrap();
    assert_eq!(
        compression_state(&reindexed, session.id).as_ref(),
        Some(&live)
    );
    shut_down(reindexed);

    let index = rusqlite::Connection::open(index_path(&paths)).unwrap();
    let removed = index
        .execute(
            r#"DELETE FROM agent_compression_links WHERE record_ordinal = (
                SELECT MIN(record_ordinal) FROM agent_compression_links
                WHERE link_json LIKE '{"delta"%')"#,
            [],
        )
        .unwrap();
    assert_eq!(removed, 1);
    drop(index);
    let rebuilt = reopen(config, &paths);
    rebuilt.core.activate_session(session.id).unwrap();
    let state = compression_state(&rebuilt, session.id).unwrap();
    let blocks = |state: &AgentCompressionState| {
        state
            .blocks
            .iter()
            .map(|block| (block.summary.clone(), block.effective_message_ids.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks(&state), blocks(&live));
    finish(rebuilt, paths);
}

/// `decompress` with `toFile` writes the block's content to the session's
/// export file under the Agent's data directory and answers with its path;
/// deleting the session removes the file.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn decompress_to_a_file_writes_under_the_session_and_goes_with_it() {
    let first = "The first answer is long. ".repeat(1_000);
    let server = FakeServer::start(
        [
            with_usage(assistant_events(&first), 1_000),
            with_usage(
                call_events(
                    "fold-1",
                    "compress",
                    &json!({ "content": format!("m00002–m00002 First answer\n{SUMMARY}") }),
                ),
                1_000,
            ),
            with_usage(
                call_events(
                    "export",
                    "decompress",
                    &json!({ "blockId": "b1", "full": true, "toFile": "/somewhere/else.md" }),
                ),
                1_000,
            ),
            with_usage(assistant_events("Written."), 1_000),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths) = service_with("compression-to-file", |_, config| {
        configure(config, &server.base_url)
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    for prompt in ["Hello.", "Fold it, then export it."] {
        service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
    }

    let requests = server.requests();
    let last: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    let output = last["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["call_id"] == "export" && item["type"] == "function_call_output")
        .map(|item| item["output"][0]["text"].as_str().unwrap().to_owned())
        .unwrap();
    assert!(
        output.contains(
            "</acp>\n[Block b1 content — 1 item(s), full]\nContent (26012 chars) written to: "
        ),
        "{output}"
    );
    let path = output
        .split_once("written to: ")
        .and_then(|(_, rest)| rest.split_once(" (this session's export file"))
        .map(|(path, _)| PathBuf::from(path))
        .unwrap();
    let exports = hanafubuki_agent_history::AgentHistoryPaths::under(history_root(&paths))
        .attachments
        .join(session.id.value().to_string())
        .join("exports");
    let expected = ResolvedPath::resolve(exports.join("decompress-b1-full.txt")).unwrap();
    assert_eq!(path, expected.native());
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written, format!("[assistant]\n{first}"));
    assert!(!last.to_string().contains(&first));
    assert!(!Path::new("/somewhere/else.md").exists());

    let before = service.snapshot(session.id).unwrap();
    service
        .delete_session_confirmed(session.id, before.generation, before.revision)
        .unwrap();
    assert!(!path.exists());
    assert!(!exports.exists());
    finish(service, paths);
}

/// The content store under `hierarchical`: a fold stores the originals it
/// covered in the session's own file, an `acp_retrieve` of a large one is
/// exported to the session's export file and answered with its path, and
/// deleting the session removes both.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn the_content_store_is_written_under_the_session_and_goes_with_it() {
    let first = "The first answer is long. ".repeat(1_000);
    let server = FakeServer::start(
        [
            with_usage(assistant_events(&first), 1_000),
            with_usage(
                call_events(
                    "fold-1",
                    "compress",
                    &json!({ "content": format!("m00002–m00002 First answer\n{SUMMARY}") }),
                ),
                1_000,
            ),
            with_usage(
                call_events("retrieve", "acp_retrieve", &json!({ "ref": "m00002" })),
                1_000,
            ),
            with_usage(assistant_events("Retrieved."), 1_000),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths) = service_with("compression-content-store", |_, config| {
        configure(config, &server.base_url);
        let mut compaction = tuned();
        compaction.hierarchical.content_store = true;
        config
            .write_settings(AgentSettingsPatch {
                compaction: Some(compaction),
                ..AgentSettingsPatch::default()
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    for prompt in ["Hello.", "Fold it, then get it back."] {
        service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
    }

    let session_root = hanafubuki_agent_history::AgentHistoryPaths::under(history_root(&paths))
        .attachments
        .join(session.id.value().to_string());
    let file = session_root.join("compression-store.json");
    let stored: hanafubuki_agent_kernel::AgentContentStore =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(stored.retrieve("m00002").unwrap().1, first);
    let requests = server.requests();
    let last: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    let output = last["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["call_id"] == "retrieve" && item["type"] == "function_call_output")
        .map(|item| item["output"][0]["text"].as_str().unwrap().to_owned())
        .unwrap();
    assert!(output.contains("[acp-retrieved #m00002 · "), "{output}");
    let path = output
        .split_once("path=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(path, _)| PathBuf::from(path))
        .unwrap();
    let expected = ResolvedPath::resolve(session_root.join("exports").join("m00002.txt")).unwrap();
    assert_eq!(path, expected.native());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    assert!(!last.to_string().contains(&first));

    let before = service.snapshot(session.id).unwrap();
    service
        .delete_session_confirmed(session.id, before.generation, before.revision)
        .unwrap();
    assert!(!file.exists());
    assert!(!path.exists());
    finish(service, paths);
}

/// A `standard` session writes no compression state at all.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_standard_session_writes_no_compression_state() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 2]).await;
    let (service, paths) = configured_service("compression-standard", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    for prompt in ["One.", "Two."] {
        service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
    }
    assert!(!rollouts(&paths).contains("\"compression"));
    let index = rusqlite::Connection::open(index_path(&paths)).unwrap();
    let links: i64 = index
        .query_row("SELECT COUNT(*) FROM agent_compression_links", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(links, 0);
    drop(index);
    finish(service, paths);
}

fn compression_state(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> Option<AgentCompressionState> {
    service.core.test_compression_state(session_id).unwrap()
}

fn history_root(paths: &TestPaths) -> PathBuf {
    paths.data_dir.join("agent-test")
}

fn index_path(paths: &TestPaths) -> PathBuf {
    hanafubuki_agent_history::AgentHistoryPaths::under(history_root(paths)).sqlite_database
}

/// Every rollout's text, one after the other.
fn rollouts(paths: &TestPaths) -> String {
    fn collect(directory: &Path, text: &mut String) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(&path, text);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                text.push_str(&std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut text = String::new();
    collect(&history_root(paths), &mut text);
    text
}

/// A new Agent over `paths`' storage, as after a restart.
fn reopen(config: AgentConfig, paths: &TestPaths) -> AgentApplicationService {
    AgentApplicationService::new_with_host(
        config,
        test_host(paths),
        AgentStoragePaths::under(history_root(paths)),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap()
}

fn shut_down(service: AgentApplicationService) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// The `[compaction]` table the sessions run with: `hierarchical`, only the
/// newest message kept out of a fold.
pub(super) fn tuned() -> AgentCompactionSettings {
    let mut settings = AgentCompactionSettings {
        strategy: AgentCompactionStrategy::Hierarchical,
        ..AgentCompactionSettings::default()
    };
    settings.hierarchical.preserve_recent_messages = 1;
    settings.hierarchical.preserve_recent_tokens = 0;
    settings.hierarchical.min_compress_range_chars = 1_000;
    settings
}

pub(super) fn configure(config: &AgentConfig, base_url: &str) {
    configure_test_provider(config, base_url, None);
    config
        .mutate_providers(1, |registry| {
            registry.model_policy.context_window_tokens = Some(100_000);
            Ok(())
        })
        .unwrap();
    config
        .write_settings(AgentSettingsPatch {
            compaction: Some(tuned()),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
}

/// A call of `name` with `arguments`, as `call_id`.
pub(super) fn call_events(call_id: &str, name: &str, arguments: &Value) -> Vec<u8> {
    String::from_utf8(tool_events(name, &arguments.to_string()))
        .unwrap()
        .replace("call-tool", call_id)
        .into_bytes()
}

/// `events` with the provider's count of the request's input tokens.
pub(super) fn with_usage(events: Vec<u8>, input_tokens: u64) -> Vec<u8> {
    let usage = format!("\"usage\":{{\"input_tokens\":{input_tokens},\"output_tokens\":1}}");
    String::from_utf8(events)
        .unwrap()
        .replace(
            "\"end_turn\":true}",
            &format!("\"end_turn\":true,{usage}}}"),
        )
        .replace(
            "\"end_turn\":false}",
            &format!("\"end_turn\":false,{usage}}}"),
        )
        .into_bytes()
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    shut_down(service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
