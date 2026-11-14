//! Tool presenters: one per tool name, present while their plugin runs, a
//! subject bounded to one line, and the fallback display for a tool without
//! one, for one that cannot read the call, and for one that panicked.

use std::sync::{Arc, Mutex};

use hanafubuki_agent_contracts::{
    AgentOutputBlock, AgentPatchFile, AgentPatchHunkStart, AgentPatchLine, AgentPatchLineKind,
    AgentPatchOperation, AgentPatchPreview, AgentStyleSpan, AgentStyledText, AgentTextColor,
    AgentTextStyle, AgentToolDisplay, AgentToolDisplayMetadata, AgentToolPresenter,
    ArgumentSubjects, ExtensionPoint, PluginFault, PluginFaults, Register, RegistrationError,
    RegistryService, ToolPresenterRegistry, MAX_TOOL_SUBJECT_CHARS,
};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use kardia_core::{Context, Fiber, FiberState, Inject, PluginDef, Service};
use serde_json::{json, Value};

struct Echo {
    tools: &'static [&'static str],
    panics: bool,
}

impl AgentToolPresenter for Echo {
    fn tools(&self) -> &[&'static str] {
        self.tools
    }

    fn subject(&self, _tool: &str, arguments: &Value) -> Option<String> {
        if self.panics {
            panic!("echo broke");
        }
        arguments.get("text")?.as_str().map(str::to_string)
    }

    fn display(
        &self,
        _tool: &str,
        arguments: &Value,
        _output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        if self.panics {
            panic!("echo broke");
        }
        Some(AgentToolDisplay::Text {
            text: arguments.get("text")?.as_str()?.to_string(),
        })
    }
}

async fn registry_root(faults: Arc<PluginFaults>) -> (Context, Arc<ToolPresenterRegistry>) {
    let registry = Arc::new(ToolPresenterRegistry::with_faults(faults));
    let provided = registry.clone();
    let root = Context::new();
    root.plugin_def(
        &PluginDef::new(move |ctx: Context, _| {
            let registry = provided.clone();
            async move {
                ctx.provide_arc(registry)?;
                Ok(())
            }
        })
        .provides([ToolPresenterRegistry::NAME]),
        Value::Null,
    )
    .unwrap()
    .settled()
    .await
    .unwrap();
    (root, registry)
}

async fn presenting(
    root: &Context,
    name: &'static str,
    presenter: Arc<dyn AgentToolPresenter>,
) -> Fiber {
    let plugin = PluginDef::new(move |ctx: Context, _| {
        let presenter = presenter.clone();
        async move {
            ctx.get::<ToolPresenterRegistry>()?.register(presenter)?;
            Ok(())
        }
    })
    .name(name)
    .inject(Inject::new().with(ToolPresenterRegistry::NAME));
    let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
    let _ = fiber.settled().await;
    fiber
}

fn structured(value: Value) -> ToolOutput {
    ToolOutput::structured(value, false)
}

#[tokio::test]
async fn a_presenter_presents_its_tools_while_its_plugin_runs() {
    let (root, registry) = registry_root(Arc::default()).await;
    let fiber = presenting(
        &root,
        "echo",
        Arc::new(Echo {
            tools: &["echo"],
            panics: false,
        }),
    )
    .await;
    assert_eq!(fiber.state(), FiberState::Active);
    let arguments = json!({ "text": "hello" });
    let output = structured(json!({ "ok": true }));

    assert_eq!(
        registry.subject("echo", &arguments).as_deref(),
        Some("hello")
    );
    assert_eq!(
        registry.display("echo", &arguments, &output, &Default::default()),
        AgentToolDisplay::Text {
            text: "hello".to_string()
        }
    );
    // Another tool, and the same one once the plugin stopped, take the
    // fallback.
    assert_eq!(registry.subject("other", &arguments), None);
    fiber.dispose().await.unwrap();
    assert_eq!(registry.subject("echo", &arguments), None);
    assert_eq!(
        registry.display("echo", &arguments, &output, &Default::default()),
        AgentToolDisplay::Fields {
            rows: vec![("ok".to_string(), "true".to_string())]
        }
    );
}

#[tokio::test]
async fn a_second_presenter_of_a_tool_fails_its_plugin() {
    let (root, registry) = registry_root(Arc::default()).await;
    let first = presenting(
        &root,
        "first",
        Arc::new(Echo {
            tools: &["echo"],
            panics: false,
        }),
    )
    .await;
    assert_eq!(first.state(), FiberState::Active);
    let candidate: Arc<dyn AgentToolPresenter> = Arc::new(Echo {
        tools: &["other", "echo"],
        panics: false,
    });
    let existing = registry.registry().snapshot();
    let existing = existing.items().collect::<Vec<_>>();
    assert!(matches!(
        ToolPresenterRegistry::admit(&existing, &candidate),
        Err(RegistrationError::Duplicate { identity, .. }) if identity == "echo"
    ));
    let second = presenting(&root, "second", candidate).await;
    assert_ne!(second.state(), FiberState::Active);
}

#[tokio::test]
async fn a_panicking_presenter_shows_no_subject_and_the_fallback() {
    let faults = Arc::new(PluginFaults::default());
    let reported = Arc::new(Mutex::new(Vec::new()));
    let sink = reported.clone();
    faults.set_handler(move |fault| sink.lock().unwrap().push(fault));
    let (root, registry) = registry_root(faults).await;
    let _fiber = presenting(
        &root,
        "broken",
        Arc::new(Echo {
            tools: &["echo"],
            panics: true,
        }),
    )
    .await;
    let output = ToolOutput {
        content: vec![
            ToolOutputContent::Text {
                text: "first".to_string(),
            },
            ToolOutputContent::Text {
                text: "second".to_string(),
            },
        ],
        structured_content: None,
        is_error: false,
    };

    assert_eq!(registry.subject("echo", &json!({ "text": "x" })), None);
    assert_eq!(
        registry.display("echo", &json!({}), &output, &Default::default()),
        AgentToolDisplay::Content {
            blocks: vec![
                AgentOutputBlock::Text {
                    text: "first".to_string()
                },
                AgentOutputBlock::Text {
                    text: "second".to_string()
                },
            ]
        }
    );
    assert_eq!(
        reported.lock().unwrap().first(),
        Some(&PluginFault {
            plugin: "broken".to_string(),
            point: ExtensionPoint::ToolPresenter.into(),
            message: "echo broke".to_string(),
        })
    );
}

#[tokio::test]
async fn a_subject_is_its_first_line_and_bounded() {
    let (root, registry) = registry_root(Arc::default()).await;
    let _fiber = presenting(
        &root,
        "subjects",
        Arc::new(ArgumentSubjects::new(&[("view", "path"), ("run", "cmd")])),
    )
    .await;
    let long = "x".repeat(MAX_TOOL_SUBJECT_CHARS + 10);

    assert_eq!(
        registry
            .subject("view", &json!({ "path": "a.png" }))
            .as_deref(),
        Some("a.png")
    );
    assert_eq!(
        registry
            .subject("run", &json!({ "cmd": "\n  cargo build\ncargo test" }))
            .as_deref(),
        Some("cargo build…")
    );
    let bounded = registry.subject("run", &json!({ "cmd": long })).unwrap();
    assert_eq!(bounded.chars().count(), MAX_TOOL_SUBJECT_CHARS + 1);
    assert!(bounded.ends_with('…'));
    assert_eq!(registry.subject("run", &json!({ "cmd": "  " })), None);
    assert_eq!(registry.subject("view", &json!({ "path": 3 })), None);
}

#[test]
fn the_fallback_shows_structured_keys_before_text_notes() {
    let mut output = structured(json!({ "path": "a", "nested": { "x": 1 }, "n": null }));
    output.content.push(ToolOutputContent::Text {
        text: "limit clamped".to_string(),
    });

    assert_eq!(
        AgentToolDisplay::fallback(&output),
        AgentToolDisplay::Fields {
            rows: vec![
                ("n".to_string(), "null".to_string()),
                ("nested".to_string(), "…".to_string()),
                ("path".to_string(), "a".to_string()),
            ]
        }
    );
    assert_eq!(
        AgentToolDisplay::fallback(&structured(json!("done"))),
        AgentToolDisplay::Text {
            text: "done".to_string()
        }
    );
}

/// An MCP result: its structured content beside items that repeat it, and
/// media the interface describes without carrying.
#[test]
fn the_fallback_shows_an_mcp_result_item_by_item_and_media_by_size() {
    // 6 MB of image data, as a server may return.
    let image = "A".repeat(8 * 1024 * 1024);
    let output = ToolOutput {
        content: vec![
            ToolOutputContent::Text {
                text: r#"{"temperature":21}"#.to_string(),
            },
            ToolOutputContent::Image {
                mime_type: "image/png".to_string(),
                data: image,
                detail: None,
            },
            ToolOutputContent::Audio {
                mime_type: "audio/wav".to_string(),
                data: "AAAA".to_string(),
            },
            ToolOutputContent::ResourceLink {
                name: "report".to_string(),
                uri: "file:///tmp/report.md".to_string(),
                mime_type: Some("text/markdown".to_string()),
            },
            ToolOutputContent::EmbeddedResource {
                uri: "file:///tmp/notes.txt".to_string(),
                mime_type: Some("text/plain".to_string()),
                text: Some("notes".to_string()),
                data: None,
            },
            ToolOutputContent::EmbeddedResource {
                uri: "file:///tmp/blob.bin".to_string(),
                mime_type: None,
                text: None,
                data: Some("AAE=".to_string()),
            },
        ],
        structured_content: Some(json!({ "temperature": 21 })),
        is_error: false,
    };

    let display = AgentToolDisplay::fallback(&output);
    assert_eq!(
        display,
        AgentToolDisplay::Content {
            blocks: vec![
                AgentOutputBlock::Text {
                    text: r#"{"temperature":21}"#.to_string()
                },
                AgentOutputBlock::Image {
                    mime_type: "image/png".to_string(),
                    bytes: 6 * 1024 * 1024,
                },
                AgentOutputBlock::Audio {
                    mime_type: "audio/wav".to_string(),
                    bytes: 3,
                },
                AgentOutputBlock::Link {
                    name: "report".to_string(),
                    uri: "file:///tmp/report.md".to_string(),
                    mime_type: Some("text/markdown".to_string()),
                },
                AgentOutputBlock::Resource {
                    uri: "file:///tmp/notes.txt".to_string(),
                    mime_type: Some("text/plain".to_string()),
                    text: Some("notes".to_string()),
                    bytes: None,
                },
                AgentOutputBlock::Resource {
                    uri: "file:///tmp/blob.bin".to_string(),
                    mime_type: None,
                    text: None,
                    bytes: Some(2),
                },
            ]
        }
    );
    // What reaches an interface never carries the image.
    assert!(serde_json::to_vec(&display).unwrap().len() < 1024);
    // An empty output shows nothing; one with only structured content its
    // keys.
    let empty = ToolOutput {
        content: Vec::new(),
        structured_content: None,
        is_error: false,
    };
    assert_eq!(
        AgentToolDisplay::fallback(&empty),
        AgentToolDisplay::Content { blocks: Vec::new() }
    );
    let only_structured = ToolOutput {
        structured_content: Some(json!({ "ok": true })),
        ..empty
    };
    assert_eq!(
        AgentToolDisplay::fallback(&only_structured),
        AgentToolDisplay::Fields {
            rows: vec![("ok".to_string(), "true".to_string())]
        }
    );
}

fn patch_file(
    operation: AgentPatchOperation,
    kinds: &[AgentPatchLineKind],
    omitted_lines: usize,
) -> AgentPatchFile {
    AgentPatchFile {
        operation,
        path: "src/lib.rs".to_string(),
        moved_to: None,
        lines: kinds
            .iter()
            .map(|kind| AgentPatchLine {
                kind: *kind,
                text: String::new(),
            })
            .collect(),
        added: 0,
        removed: 0,
        omitted_lines,
        hunk_starts: Vec::new(),
    }
}

fn start(old_line: u64, new_line: u64) -> AgentPatchHunkStart {
    AgentPatchHunkStart { old_line, new_line }
}

fn hunk_starts_of(display: AgentToolDisplay) -> Vec<Vec<AgentPatchHunkStart>> {
    let AgentToolDisplay::Patch { preview } = display else {
        panic!("a patch")
    };
    preview
        .files
        .into_iter()
        .map(|file| file.hunk_starts)
        .collect()
}

#[test]
fn a_patchs_display_takes_the_chunk_starts_of_its_own_files_only() {
    use AgentPatchLineKind::{Added, Context, Hunk, Removed};
    let patch = |files| AgentToolDisplay::Patch {
        preview: AgentPatchPreview { files, error: None },
    };
    let update = patch_file(
        AgentPatchOperation::Update,
        &[Hunk, Context, Removed, Hunk, Added],
        0,
    );
    let added = patch_file(AgentPatchOperation::Add, &[Added], 0);
    let metadata = |starts| AgentToolDisplayMetadata {
        patch_hunk_starts: Some(starts),
        ..AgentToolDisplayMetadata::default()
    };
    let fits = metadata(vec![vec![start(3, 3), start(9, 8)], Vec::new()]);
    assert_eq!(
        hunk_starts_of(patch(vec![update.clone(), added.clone()]).with_metadata(&fits)),
        vec![vec![start(3, 3), start(9, 8)], Vec::new()]
    );
    // Another patch's, or none: unnumbered.
    for other in [
        metadata(vec![vec![start(3, 3)], Vec::new()]),
        metadata(vec![vec![start(3, 3), start(9, 8)]]),
        metadata(vec![vec![start(3, 3), start(9, 8)], vec![start(1, 1)]]),
        AgentToolDisplayMetadata::default(),
    ] {
        assert_eq!(
            hunk_starts_of(patch(vec![update.clone(), added.clone()]).with_metadata(&other)),
            vec![Vec::new(), Vec::new()],
            "{other:?}"
        );
    }
    // A preview that left chunks out keeps the starts of those it shows.
    let cut = patch_file(AgentPatchOperation::Update, &[Hunk, Added], 40);
    let three = metadata(vec![vec![start(1, 1), start(5, 6), start(9, 11)]]);
    assert_eq!(
        hunk_starts_of(patch(vec![cut]).with_metadata(&three)),
        vec![vec![start(1, 1), start(5, 6), start(9, 11)]]
    );
    // The metadata decodes tolerantly and stays out of an empty record.
    let json = serde_json::to_value(&fits).unwrap();
    assert_eq!(
        json,
        json!({ "patchHunkStarts": [[{ "oldLine": 3, "newLine": 3 }, { "oldLine": 9, "newLine": 8 }], []] })
    );
    assert_eq!(
        serde_json::from_value::<AgentToolDisplayMetadata>(json).unwrap(),
        fits
    );
    assert_eq!(
        serde_json::from_value::<AgentToolDisplayMetadata>(
            json!({ "patchHunkStarts": [[{ "oldLine": "three" }]] })
        )
        .unwrap(),
        AgentToolDisplayMetadata::default()
    );
}

fn command(output: &str) -> AgentToolDisplay {
    AgentToolDisplay::Command {
        command: "ls --color=always".to_string(),
        cwd: None,
        output: output.to_string(),
        exit_code: Some(0),
        duration_millis: None,
        truncated: false,
        styled: None,
    }
}

fn red_styles(text_bytes: usize, end: usize) -> AgentToolDisplayMetadata {
    AgentToolDisplayMetadata {
        styled_output: Some(AgentStyledText {
            text_bytes,
            spans: vec![AgentStyleSpan {
                start: 0,
                end,
                style: AgentTextStyle {
                    fg: Some(AgentTextColor::Indexed(1)),
                    ..AgentTextStyle::default()
                },
            }],
        }),
        ..AgentToolDisplayMetadata::default()
    }
}

#[test]
fn a_commands_display_takes_the_styles_of_its_own_output_only() {
    let metadata = red_styles(10, 3);
    let AgentToolDisplay::Command { styled, .. } = command("src\nnotes\n").with_metadata(&metadata)
    else {
        unreachable!()
    };
    assert_eq!(styled, metadata.styled_output);
    // An output the kernel bounded, or another one, shows plain.
    for other in ["src\nnot", "src\nnotes\n and more", "模型\n"] {
        let AgentToolDisplay::Command { styled, .. } = command(other).with_metadata(&metadata)
        else {
            unreachable!()
        };
        assert_eq!(styled, None, "{other:?}");
    }
    // Any other display is left as it is.
    let text = AgentToolDisplay::Text {
        text: "src\nnotes\n".to_string(),
    };
    assert_eq!(text.clone().with_metadata(&metadata), text);
}

#[test]
fn display_metadata_is_absent_when_empty_and_decodes_tolerantly() {
    assert_eq!(
        serde_json::to_value(AgentToolDisplayMetadata::default()).unwrap(),
        json!({})
    );
    assert_eq!(
        serde_json::to_value(command("x")).unwrap().get("styled"),
        None
    );
    let metadata = red_styles(3, 3);
    let json = serde_json::to_value(&metadata).unwrap();
    assert_eq!(
        serde_json::from_value::<AgentToolDisplayMetadata>(json).unwrap(),
        metadata
    );
    for broken in [
        json!({ "styledOutput": "red" }),
        json!({ "styledOutput": { "textBytes": -1, "spans": [] } }),
        json!({ "styledOutput": { "textBytes": 3, "spans": [{ "start": 0 }] } }),
        json!({ "futureField": 1 }),
    ] {
        assert_eq!(
            serde_json::from_value::<AgentToolDisplayMetadata>(broken.clone()).unwrap(),
            AgentToolDisplayMetadata::default(),
            "{broken}"
        );
    }
}

struct Exec;

impl AgentToolPresenter for Exec {
    fn tools(&self) -> &[&'static str] {
        &["exec"]
    }

    fn display(&self, _: &str, _: &Value, _: &ToolOutput) -> Option<AgentToolDisplay> {
        Some(command("red"))
    }
}

#[tokio::test]
async fn the_registry_adds_the_metadata_to_a_presenters_command() {
    let (root, registry) = registry_root(Arc::default()).await;
    let _fiber = presenting(&root, "exec", Arc::new(Exec)).await;
    let output = structured(json!({}));
    let metadata = red_styles(3, 3);
    let AgentToolDisplay::Command { styled, .. } =
        registry.display("exec", &json!({}), &output, &metadata)
    else {
        panic!("a command")
    };
    assert_eq!(styled, metadata.styled_output);
}
