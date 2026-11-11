use std::fs;
use std::path::PathBuf;

use super::*;
use crate::{AgentConfig, ConfigOverrides, AGENT_SETTINGS_FILE};

/// A configuration directory and a project, the user's `config.toml` as
/// given, `<PROJECT>` in it replaced by the project's path.
struct Fixture {
    _root: tempfile::TempDir,
    config_dir: PathBuf,
    project: PathBuf,
}

fn setup(user: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join("config");
    let project = root.path().join("project");
    fs::create_dir_all(config_dir.join(ROLE_DIR)).unwrap();
    fs::create_dir_all(project.join(PROJECT_CONFIG_DIR).join(ROLE_DIR)).unwrap();
    let user = user.replace(
        "<PROJECT>",
        &project.display().to_string().replace('\\', "\\\\"),
    );
    fs::write(config_dir.join(AGENT_SETTINGS_FILE), user).unwrap();
    Fixture {
        _root: root,
        config_dir,
        project,
    }
}

impl Fixture {
    fn user_role(&self, file: &str, text: &str) {
        let path = self.config_dir.join(ROLE_DIR).join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn project_role(&self, file: &str, text: &str) {
        let dir = self.project.join(PROJECT_CONFIG_DIR);
        fs::write(dir.join(ROLE_DIR).join(file), text).unwrap();
    }

    fn sources(&self, profile: Option<&str>) -> Vec<RoleSource> {
        AgentConfig::layered(
            &self.config_dir,
            ConfigLayers {
                profile: profile.map(str::to_string),
                project: Some(self.project.clone()),
                overrides: ConfigOverrides::default(),
            },
        )
        .role_sources()
        .unwrap()
    }

    fn user(&self) -> RoleSource {
        self.sources(None).remove(0)
    }
}

fn names(source: &RoleSource) -> Vec<&str> {
    source
        .definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect()
}

fn only(source: &RoleSource) -> &RoleDefinition {
    assert_eq!(source.definitions.len(), 1, "{source:#?}");
    &source.definitions[0]
}

#[test]
fn a_role_file_reads_every_key() {
    let fixture = setup("");
    fixture.user_role(
        "explorer.toml",
        r#"
description = "Reads the code."
extends = "default"
instructions = "Answer with paths."
instructionsMode = "replace"
model = "gpt-5.4-mini"
reasoningEffort = "low"
permissionModeCap = "auto_review"
spawnable = false

[tools]
allow = ["read_file", "plugin:web"]
deny = ["fetch_web_page"]

[plugins]
off = ["titles"]
"#,
    );
    let source = fixture.user();
    let role = only(&source);
    assert_eq!(role.name, "explorer");
    assert_eq!(role.description.as_deref(), Some("Reads the code."));
    assert_eq!(role.extends.as_deref(), Some("default"));
    assert_eq!(role.instructions.as_deref(), Some("Answer with paths."));
    assert_eq!(role.instructions_mode, InstructionsMode::Replace);
    assert_eq!(role.model.as_deref(), Some("gpt-5.4-mini"));
    assert_eq!(role.reasoning_effort, Some(ModelReasoningEffort::Low));
    assert_eq!(
        role.permission_mode_cap,
        Some(AgentPermissionMode::AutoReview)
    );
    assert_eq!(role.spawnable, Some(false));
    assert_eq!(
        role.tools_allow,
        Some(vec!["read_file".to_string(), "plugin:web".to_string()])
    );
    assert_eq!(role.tools_deny, vec!["fetch_web_page".to_string()]);
    assert_eq!(role.plugins_off, vec!["titles".to_string()]);
    assert_eq!(role.unusable, None);
    assert!(role.issues.is_empty(), "{:?}", role.issues);
    assert!(matches!(&role.origin, RoleOrigin::File { path } if path.ends_with("explorer.toml")));
}

#[test]
fn a_key_that_cannot_be_used_is_left_out_with_an_issue() {
    let fixture = setup("");
    fixture.user_role(
        "tolerant.toml",
        r#"
description = "Kept."
model = 4
reasoningEffort = "very much"
instructionsMode = "prepend"
spawnable = "yes"
instructions = "   "
colour = "blue"

[tools]
allow = ["read_file", 3]
also = true
"#,
    );
    let source = fixture.user();
    let role = only(&source);
    assert_eq!(role.description.as_deref(), Some("Kept."));
    assert_eq!(role.model, None);
    assert_eq!(role.reasoning_effort, None);
    assert_eq!(role.instructions_mode, InstructionsMode::Append);
    assert_eq!(role.spawnable, None);
    assert_eq!(role.instructions, None);
    assert_eq!(role.tools_allow, Some(vec!["read_file".to_string()]));
    assert_eq!(role.unusable, None);
    let issues = role.issues.join("\n");
    for key in [
        "`model`",
        "`reasoningEffort`",
        "`instructionsMode`",
        "`spawnable`",
        "`instructions`",
        "`colour`",
        "`tools.allow[1]`",
        "`tools.also`",
    ] {
        assert!(issues.contains(key), "{key} in {issues}");
    }
}

#[test]
fn a_narrowing_key_that_cannot_be_read_makes_the_role_unusable() {
    for text in [
        "extends = 3",
        "permissionModeCap = \"read-only\"",
        "tools = \"read_file\"",
        "tools.allow = \"read_file\"",
        "tools.deny = 1",
        "plugins.off = \"titles\"",
    ] {
        let fixture = setup("");
        fixture.user_role("narrow.toml", &format!("description = \"d\"\n{text}\n"));
        let source = fixture.user();
        let role = only(&source);
        let why = role.unusable.as_deref().unwrap_or_else(|| panic!("{text}"));
        assert!(why.contains("would allow more"), "{why}");
    }
}

#[test]
fn codex_role_files_work_through_the_aliases() {
    let fixture = setup("");
    fixture.user_role(
        "reviewer.toml",
        r#"
name = "reviewer"
description = "Reviews."
developer_instructions = "Be terse."
model = "gpt-5.4"
model_reasoning_effort = "high"
sandbox_mode = "read-only"
nickname_candidates = ["Ada"]
"#,
    );
    let source = fixture.user();
    let role = only(&source);
    assert_eq!(role.instructions.as_deref(), Some("Be terse."));
    assert_eq!(role.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(role.reasoning_effort, Some(ModelReasoningEffort::High));
    let issues = role.issues.join("\n");
    assert!(issues.contains("`sandbox_mode`") && issues.contains("Codex setting"));
    assert!(issues.contains("`nickname_candidates`"));

    let fixture = setup("");
    fixture.user_role(
        "both.toml",
        "description = \"d\"\ninstructions = \"ours\"\ndeveloper_instructions = \"theirs\"\n",
    );
    let source = fixture.user();
    let role = only(&source);
    assert_eq!(role.instructions.as_deref(), Some("ours"));
    assert!(role.issues[0].contains("`developer_instructions` is ignored"));
}

#[test]
fn a_file_is_named_by_its_name_or_its_stem_and_kept_when_unreadable() {
    let fixture = setup("");
    fixture.user_role("a.toml", "name = \"zeta\"\ndescription = \"d\"\n");
    fixture.user_role("b.toml", "name = \"bad:name\"\ndescription = \"d\"\n");
    fixture.user_role("c.toml", "description = ");
    fixture.user_role("nested/d.toml", "description = \"d\"\n");
    fixture.user_role("notes.md", "not a role");
    fixture.user_role("two words.toml", "description = \"d\"\n");
    let source = fixture.user();
    assert_eq!(names(&source), ["zeta", "b", "c", "d"]);
    assert!(source.definitions[1].issues[0].contains("stem is used"));
    let broken = &source.definitions[2];
    assert!(broken
        .unusable
        .as_deref()
        .unwrap()
        .contains("not valid TOML"));
    assert_eq!(source.issues.len(), 1, "{:?}", source.issues);
    assert!(source.issues[0].contains("two words.toml"));
}

#[test]
fn entries_come_before_files_and_read_their_config_file() {
    let fixture = setup(
        r#"
[agents.zeta]
description = "Inline."
tools.deny = ["exec_command"]

[agents.explorer]
configFile = "agents/explorer.toml"
model = "inline-model"

[agents.missing]
configFile = "agents/none.toml"

[agents.max_threads]
"#,
    );
    let codex = setup("[agents]\nmax_threads = 4\n");
    fixture.user_role(
        "explorer.toml",
        "name = \"other\"\ndescription = \"From the file.\"\nmodel = \"file-model\"\n",
    );
    fixture.user_role("alpha.toml", "description = \"A file.\"\n");
    let source = fixture.user();
    // `[agents]` entries by name, then the files by path; the file an
    // entry names is the entry's, not the directory's.
    assert_eq!(
        names(&source),
        ["explorer", "max_threads", "missing", "zeta", "alpha"]
    );
    let explorer = &source.definitions[0];
    assert_eq!(explorer.description.as_deref(), Some("From the file."));
    assert_eq!(explorer.model.as_deref(), Some("inline-model"));
    assert!(explorer.issues[0].contains("the entry's `explorer` is used"));
    let RoleOrigin::Table {
        key, config_file, ..
    } = &explorer.origin
    else {
        panic!("{:?}", explorer.origin);
    };
    assert_eq!(key, "agents.explorer");
    assert!(config_file.as_ref().unwrap().ends_with("explorer.toml"));
    assert!(source.definitions[2]
        .unusable
        .as_deref()
        .unwrap()
        .contains("cannot be read"));
    assert_eq!(
        source.definitions[3].tools_deny,
        vec!["exec_command".to_string()]
    );

    let source = codex.user();
    assert!(source.definitions.is_empty());
    assert!(source.issues[0].contains("agents.max_threads"));
}

#[test]
fn the_profile_and_a_trusted_project_are_sources_of_their_own() {
    let fixture = setup(
        r#"
[agents.mine]
description = "User."

[profiles.work.agents.mine]
description = "Profile."

[projects."<PROJECT>"]
trustLevel = "trusted"
"#,
    );
    fixture.project_role("mine.toml", "description = \"Project file.\"\n");
    fs::write(
        ConfigLayers::project_file(&fixture.project),
        "[agents.local]\ndescription = \"Project entry.\"\n",
    )
    .unwrap();
    let sources = fixture.sources(Some("work"));
    let layers: Vec<_> = sources.iter().map(|source| source.layer.clone()).collect();
    assert_eq!(
        layers,
        [
            RoleLayer::User,
            RoleLayer::Profile {
                name: "work".into()
            },
            RoleLayer::Project
        ]
    );
    assert_eq!(names(&sources[1]), ["mine"]);
    assert_eq!(
        sources[1].definitions[0].description.as_deref(),
        Some("Profile.")
    );
    let RoleOrigin::Table { key, .. } = &sources[1].definitions[0].origin else {
        panic!();
    };
    assert_eq!(key, "profiles.work.agents.mine");
    assert_eq!(names(&sources[2]), ["local", "mine"]);
}

#[test]
fn a_trusted_project_without_a_config_file_has_its_role_files_read() {
    let fixture = setup("[projects.\"<PROJECT>\"]\ntrustLevel = \"trusted\"\n");
    fixture.project_role("theirs.toml", "description = \"Project.\"\n");
    assert!(!ConfigLayers::project_file(&fixture.project).exists());
    let sources = fixture.sources(None);
    assert_eq!(sources[1].layer, RoleLayer::Project);
    assert_eq!(names(&sources[1]), ["theirs"]);
    assert!(sources[1].issues.is_empty(), "{:?}", sources[1].issues);
}

#[test]
fn an_untrusted_project_and_a_host_configuration_read_the_user_alone() {
    let fixture = setup("[agents.mine]\ndescription = \"User.\"\n");
    fixture.project_role("theirs.toml", "description = \"Project.\"\n");
    let sources = fixture.sources(None);
    assert_eq!(sources.len(), 1);

    let host = AgentConfig::new(&fixture.config_dir)
        .role_sources()
        .unwrap();
    assert_eq!(host.len(), 1);
    assert_eq!(names(&host[0]), ["mine"]);
}

#[test]
fn the_agents_table_is_not_an_unknown_key_but_under_c_it_is() {
    let fixture = setup(
        r#"
[agents.mine]
description = "User."

[profiles.work.agents.mine]
description = "Profile."
"#,
    );
    let config = AgentConfig::layered(
        &fixture.config_dir,
        ConfigLayers {
            profile: Some("work".into()),
            project: None,
            overrides: ConfigOverrides::parse(&["agents.x.description=\"d\""]).unwrap(),
        },
    );
    let warnings = config.layer_warnings().unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("-c has the unknown key agents"),
        "{warnings:?}"
    );
}

#[test]
fn role_names_leave_the_colon_to_installed_plugins() {
    assert!(is_role_name("explorer"));
    assert!(is_role_name("deep-review_2"));
    for name in ["", "demo:explorer", "two words", "tab\t", &"a".repeat(129)] {
        assert!(!is_role_name(name), "{name:?}");
    }
}
