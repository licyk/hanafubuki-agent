//! The line numbers of an updated file's diff, end to end on the fake model
//! server: the model's patch updates two places of a file; the request after
//! it and the history's tool output carry the result as before, without the
//! positions; the display (live, and read back after a restart) has each
//! chunk's first line before and after the patch.

use super::colour::{reopen, run_approved_turn, transcript};
use super::*;
use serde_json::Value;

const ORIGINAL: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";

fn hunk_starts(display: &AgentToolDisplay) -> Vec<Vec<(u64, u64)>> {
    let AgentToolDisplay::Patch { preview } = display else {
        panic!("a patch: {display:?}");
    };
    preview
        .files
        .iter()
        .map(|file| {
            file.hunk_starts
                .iter()
                .map(|start| (start.old_line, start.new_line))
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn an_applied_patch_numbers_its_diff_without_telling_the_model() {
    // A file of the person's, outside the application's own directories.
    let directory = std::env::temp_dir().join(format!(
        "hanafubuki-agent-patch-lines-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("notes.txt");
    std::fs::write(&file, ORIGINAL).unwrap();
    let patch = format!(
        "*** Begin Patch\n*** Update File: {}\n@@\n two\n-three\n+THREE\n+3\n@@\n six\n-seven\n+SEVEN\n*** End Patch",
        file.display()
    );
    let arguments = serde_json::to_string(&json!({ "patch": patch })).unwrap();
    let server = FakeServer::start(vec![
        (tool_events("apply_patch", &arguments), Duration::ZERO),
        (assistant_events("Patched."), Duration::ZERO),
    ])
    .await;
    let (service, paths, config) = service_with_config("patch-lines", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    run_approved_turn(&service, session.id, "Patch it.").await;
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "one\ntwo\nTHREE\n3\nfour\nfive\nsix\nSEVEN\n"
    );

    // The model reads the result as before: the files it changed.
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    let output = second["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("the patch's output")
        .to_string();
    assert!(output.contains("modified"), "{output}");
    for hidden in ["hunk", "Hunk", "oldLine", "newLine"] {
        assert!(!output.contains(hidden), "{output}");
    }

    let items = transcript(&service, session.id);
    let Some(ConversationItem::ToolOutput {
        call_id,
        output,
        display_metadata,
        ..
    }) = items
        .iter()
        .find(|item| matches!(item, ConversationItem::ToolOutput { .. }))
    else {
        panic!("{items:#?}");
    };
    assert!(!serde_json::to_string(output).unwrap().contains("Line"));
    assert!(display_metadata.patch_hunk_starts.is_some());
    // `two` is line 2 before and after; `six` is line 6 before, 7 after
    // the first chunk added a line.
    let numbered = vec![vec![(2, 2), (6, 7)]];
    let shown = service.tool_display(session.id, call_id).unwrap().unwrap();
    assert_eq!(hunk_starts(shown.display.as_ref().unwrap()), numbered);
    let call_id = call_id.clone();
    // The export is the history's, as before.
    let export = service.export_session_markdown(session.id).unwrap();
    assert!(export.contains("apply_patch"), "{export}");
    for hidden in ["hunkStarts", "patchHunkStarts", "oldLine", "newLine"] {
        assert!(!export.contains(hidden), "{export}");
    }
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());

    // Read back from the history after a restart, as `/resume` does.
    let reopened = reopen(config, &paths).await;
    let shown = reopened
        .tool_display(session.id, &call_id)
        .unwrap()
        .unwrap();
    assert_eq!(hunk_starts(shown.display.as_ref().unwrap()), numbered);
    assert!(reopened
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    let _ = std::fs::remove_dir_all(directory);
}
