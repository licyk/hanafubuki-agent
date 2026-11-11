use std::fs;
use std::path::Path;

use super::*;

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// The deny list of `permissionMode: plan`, with `extra` added.
fn plan_denied(extra: &[&str]) -> Vec<String> {
    let mut denied = names(&PLAN_MODE_DENIED_TOOLS);
    denied.extend(names(extra));
    denied.sort();
    denied
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// The one agent `contents` defines, read from `agents/agent.md`.
fn agent(contents: &str) -> ClaudeAgent {
    let root = tempfile::tempdir().unwrap();
    write(&root.path().join("agents/agent.md"), contents);
    let read = read_claude_agents(&[root.path().join("agents")]);
    assert!(read.issues.is_empty(), "{:?}", read.issues);
    let [agent] = read.agents.as_slice() else {
        panic!("one agent: {:?}", read.agents);
    };
    agent.clone()
}

#[test]
fn every_key_is_read_and_the_body_is_the_instructions() {
    let agent = agent(
        "---\nname: reviewer\ndescription: Reviews a change.\ntools: Read, Grep, Glob, LS, \
         mcp__github__list_issues\nmodel: inherit\n---\n\nReview the diff.\n",
    );

    assert_eq!(agent.name.as_deref(), Some("reviewer"));
    assert_eq!(agent.description.as_deref(), Some("Reviews a change."));
    assert_eq!(agent.instructions.as_deref(), Some("Review the diff."));
    assert_eq!(
        agent.tools,
        Some(vec![
            "list_directory".to_string(),
            "mcp__github__list_issues".to_string(),
            "read_file".to_string(),
            "search_text".to_string(),
        ])
    );
    assert_eq!(agent.unusable, None);
    assert!(agent.issues.is_empty(), "{:?}", agent.issues);
}

#[test]
fn the_tool_table_maps_each_claude_tool() {
    let table = [
        ("Read", "read_file"),
        ("Grep", "search_text"),
        ("Glob", "list_directory"),
        ("LS", "list_directory"),
        ("Edit", "apply_patch"),
        ("MultiEdit", "apply_patch"),
        ("Write", "apply_patch"),
        ("Bash", "exec_command"),
        ("WebFetch", "fetch_web_page"),
        ("WebSearch", "web_search"),
        ("mcp__docs__search", "mcp__docs__search"),
    ];
    for (claude, ours) in table {
        assert_eq!(claude_tool_name(claude), Some(ours), "{claude}");
    }
    for unknown in [
        "Task",
        "TodoWrite",
        "Bash(git:*)",
        "mcp__docs",
        "mcp____x",
        "read_file",
    ] {
        assert_eq!(claude_tool_name(unknown), None, "{unknown}");
    }
}

#[test]
fn unknown_tools_are_left_out_with_an_issue() {
    let agent = agent(
        "---\ndescription: Edits.\ntools:\n  - Edit\n  - Task\n  - 3\n  - Write\n---\nEdit.\n",
    );

    assert_eq!(agent.tools, Some(vec!["apply_patch".to_string()]));
    assert_eq!(
        agent.issues,
        [
            "`tools[2]` is not text; it is left out",
            "the tool `Task` has no counterpart in this Agent; it is left out",
        ]
    );
}

#[test]
fn a_tools_list_of_unknown_tools_allows_none() {
    let agent = agent("---\ndescription: Plans.\ntools: Task, TodoWrite\n---\n");

    assert_eq!(agent.tools, Some(Vec::new()));
    assert_eq!(agent.unusable, None);
    assert_eq!(agent.issues.len(), 2, "{:?}", agent.issues);
}

#[test]
fn without_tools_nothing_is_narrowed_and_unreadable_tools_make_it_unusable() {
    assert_eq!(agent("---\ndescription: All.\n---\n").tools, None);
    assert_eq!(agent("---\ndescription: All.\ntools:\n---\n").tools, None);

    let agent = agent("---\ndescription: Odd.\ntools:\n  Read: true\n---\n");
    assert_eq!(agent.tools, None);
    assert!(agent
        .unusable
        .as_deref()
        .is_some_and(|why| why.contains("would allow every tool")));
}

#[test]
fn model_inherit_is_honoured_and_another_model_is_ignored() {
    let inherit = agent("---\ndescription: D.\nmodel: inherit\n---\n");
    assert!(inherit.issues.is_empty(), "{:?}", inherit.issues);

    let sonnet = agent("---\ndescription: D.\nmodel: sonnet\n---\n");
    assert_eq!(
        sonnet.issues,
        [
            "`model` `sonnet` is ignored: Claude's models are not this Agent's, so the model is \
          inherited"
        ]
    );
    let number = agent("---\ndescription: D.\nmodel: 4\n---\n");
    assert_eq!(number.issues, ["`model` is not text; it is ignored"]);
}

#[test]
fn a_key_of_the_wrong_type_is_left_out_alone_and_others_are_ignored() {
    let agent = agent(
        "---\nname: [reviewer]\ndescription: 42\ncolor: blue\ndisallowedTools: Bash\ntools: Read\n\
         ---\nRead it.\n",
    );

    assert_eq!(agent.name, None);
    assert_eq!(agent.description, None);
    assert_eq!(agent.tools, Some(vec!["read_file".to_string()]));
    assert_eq!(agent.denied_tools, ["exec_command"]);
    assert_eq!(agent.instructions.as_deref(), Some("Read it."));
    assert_eq!(
        agent.issues,
        [
            "`name` is not text; the file's name is used",
            "`description` is not text; it is ignored",
        ]
    );
}

#[test]
fn a_description_holding_a_colon_is_read() {
    let agent = agent(
        "---\nname: helper\ndescription: Use it when: the user asks. Example: <example>x</example>\n\
         tools: Read\n---\n",
    );

    assert_eq!(
        agent.description.as_deref(),
        Some("Use it when: the user asks. Example: <example>x</example>")
    );
    assert_eq!(agent.tools, Some(vec!["read_file".to_string()]));
}

#[test]
fn files_without_frontmatter_or_yaml_are_issues_and_paths_are_read_once() {
    let root = tempfile::tempdir().unwrap();
    let agents = root.path().join("agents");
    write(&agents.join("plain.md"), "Just text.\n");
    write(
        &agents.join("broken.md"),
        "---\ndescription: [unclosed\n---\n",
    );
    write(
        &agents.join("nested/deep.md"),
        "---\ndescription: Deep.\n---\n",
    );
    write(&agents.join("notes.txt"), "not an agent\n");
    write(&root.path().join("one.md"), "---\ndescription: One.\n---\n");

    let read = read_claude_agents(&[
        agents.clone(),
        agents.join("nested/deep.md"),
        root.path().join("one.md"),
        root.path().join("missing"),
    ]);

    let names: Vec<_> = read
        .agents
        .iter()
        .map(|agent| agent.file.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["deep.md", "one.md"]);
    assert_eq!(read.issues.len(), 3, "{:?}", read.issues);
    assert!(read.issues[0].contains("missing"), "{:?}", read.issues);
    assert!(read
        .issues
        .iter()
        .any(|issue| issue.contains("broken.md") && issue.contains("is not YAML")));
    assert!(read
        .issues
        .iter()
        .any(|issue| issue.contains("plain.md") && issue.contains("has no frontmatter")));
}

#[test]
fn disallowed_tools_are_denied_through_the_tool_table() {
    let text = agent(
        "---\ndescription: D.\ndisallowedTools: Write, Bash, mcp__github__create_issue, Edit\n---\n",
    );
    assert_eq!(
        text.denied_tools,
        ["apply_patch", "exec_command", "mcp__github__create_issue"]
    );
    assert_eq!(text.tools, None);
    assert_eq!(text.unusable, None);
    assert!(text.issues.is_empty(), "{:?}", text.issues);

    let list = agent("---\ndescription: D.\ndisallowedTools:\n  - WebFetch\n  - WebSearch\n---\n");
    assert_eq!(list.denied_tools, ["fetch_web_page", "web_search"]);
    assert!(list.issues.is_empty(), "{:?}", list.issues);
}

#[test]
fn a_denied_mcp_server_denies_every_tool_of_it() {
    let agent = agent("---\ndescription: D.\ndisallowedTools: mcp__github, mcp__\n---\n");
    assert_eq!(agent.denied_tools, ["mcp__github__*"]);
    assert_eq!(agent.issues.len(), 1, "{:?}", agent.issues);
}

#[test]
fn a_denied_tool_without_a_counterpart_or_not_text_is_left_out_with_an_issue() {
    let agent =
        agent("---\ndescription: D.\ndisallowedTools:\n  - NotebookEdit\n  - 7\n  - Read\n---\n");

    assert_eq!(agent.denied_tools, ["read_file"]);
    assert_eq!(agent.unusable, None);
    assert_eq!(
        agent.issues,
        [
            "`disallowedTools[1]` is not text; it is left out",
            "the denied tool `NotebookEdit` has no counterpart in this Agent, so there is none to \
             deny; it is left out",
        ]
    );
}

#[test]
fn a_denied_tool_with_a_specifier_denies_the_whole_tool() {
    let agent = agent("---\ndescription: D.\ndisallowedTools: Bash(git push:*), Task(x)\n---\n");

    assert_eq!(agent.denied_tools, ["exec_command"]);
    assert_eq!(
        agent.issues,
        [
            "the denied tool `Bash(git push:*)` names some of a tool's calls, which a role cannot \
             say; the whole `exec_command` is denied",
            "the denied tool `Task(x)` has no counterpart in this Agent, so there is none to deny; \
             it is left out",
        ]
    );
}

#[test]
fn unreadable_disallowed_tools_make_it_unusable() {
    assert!(agent("---\ndescription: D.\ndisallowedTools:\n---\n")
        .denied_tools
        .is_empty());

    let agent = agent("---\ndescription: D.\ndisallowedTools:\n  Bash: true\n---\n");
    assert!(agent.denied_tools.is_empty());
    assert_eq!(
        agent.unusable.as_deref(),
        Some(
            "`disallowedTools` is not text or a list, and leaving it out would allow the tools \
             it denies"
        )
    );
}

#[test]
fn each_permission_mode_is_a_cap() {
    let cap = |mode: &str| {
        let agent = agent(&format!(
            "---\ndescription: D.\npermissionMode: {mode}\n---\n"
        ));
        assert!(
            agent.denied_tools.is_empty(),
            "{mode}: {:?}",
            agent.denied_tools
        );
        assert_eq!(agent.unusable, None, "{mode}");
        (agent.permission_mode_cap, agent.issues)
    };

    let none: Vec<String> = Vec::new();
    assert_eq!(
        cap("default"),
        (Some(AgentPermissionMode::Manual), none.clone())
    );
    assert_eq!(
        cap("acceptEdits"),
        (Some(AgentPermissionMode::AutoReview), none.clone())
    );
    assert_eq!(
        cap("bypassPermissions"),
        (Some(AgentPermissionMode::FullAccess), none)
    );
    assert_eq!(
        cap("dontAsk"),
        (
            Some(AgentPermissionMode::Manual),
            names(&[
                "`permissionMode` `dontAsk` denies what it does not allow, which a cap \
                     cannot say; the role runs in manual"
            ])
        )
    );
    assert_eq!(
        cap("sometimes"),
        (
            Some(AgentPermissionMode::Manual),
            names(&[
                "`permissionMode` `sometimes` is not one of Claude Code's modes; the role \
                     runs in manual"
            ])
        )
    );
    assert_eq!(
        agent("---\ndescription: D.\n---\n").permission_mode_cap,
        None
    );
}

#[test]
fn a_permission_mode_that_is_not_text_makes_it_unusable() {
    let agent = agent("---\ndescription: D.\npermissionMode: [plan]\n---\n");

    assert_eq!(agent.permission_mode_cap, None);
    assert!(agent.denied_tools.is_empty());
    assert_eq!(
        agent.unusable.as_deref(),
        Some("`permissionMode` is not text, and leaving it out would allow every permission mode")
    );
}

#[test]
fn plan_without_tools_denies_the_write_tools_and_every_mcp_tool() {
    let agent =
        agent("---\ndescription: D.\npermissionMode: plan\ndisallowedTools: WebFetch\n---\n");

    assert_eq!(agent.permission_mode_cap, Some(AgentPermissionMode::Manual));
    assert_eq!(
        agent.denied_tools,
        plan_denied(&["fetch_web_page", "mcp__*"])
    );
    assert!(agent.issues.is_empty(), "{:?}", agent.issues);
}

#[test]
fn plan_with_tools_denies_the_write_tools_and_keeps_the_listed_mcp_tools() {
    let agent = agent(
        "---\ndescription: D.\ntools: Read, Bash, mcp__github__list_issues\npermissionMode: plan\n\
         ---\n",
    );

    assert_eq!(agent.permission_mode_cap, Some(AgentPermissionMode::Manual));
    assert_eq!(
        agent.tools,
        Some(names(&[
            "exec_command",
            "mcp__github__list_issues",
            "read_file"
        ]))
    );
    // The allow list admits only the MCP tool it lists; `exec_command` is
    // denied, as Plan mode removes it.
    assert_eq!(agent.denied_tools, plan_denied(&[]));
    assert!(agent.issues.is_empty(), "{:?}", agent.issues);
}
