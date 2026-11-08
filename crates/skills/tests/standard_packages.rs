//! The catalog against Skill packages as they are actually shipped.
//!
//! Every case here is a shape a real, standard package has: a multi-megabyte
//! template beside the instructions, a `SKILL.md` far longer than any one
//! turn's budget, a licence and a readme, a dot-directory from the tool that
//! packaged it. The contract is that none of them costs the user the Skill.
//!
//! Set `HANAFUBUKI_SKILL_SMOKE_ROOT` to a directory of real packages — a Codex
//! plugin cache, for example — to run the same assertions against them.

use std::path::{Path, PathBuf};

use hanafubuki_agent_skills::{
    SkillIssueSeverity, SkillRoots, SkillService, MAX_TURN_DOCUMENT_BYTES,
};

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    service: SkillService,
}

impl Fixture {
    fn over(repo: Vec<PathBuf>) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_path_buf();
        let service = SkillService::new(SkillRoots {
            system: root.join("system"),
            user: root.join("user"),
            install_bundled_system_skills: false,
            state_file: root.join("state.json"),
        })
        .unwrap();
        service.set_repo_roots(repo).unwrap();
        Self {
            _temporary: temporary,
            root,
            service,
        }
    }

    fn new() -> Self {
        let fixture = Self::over(Vec::new());
        fixture
            .service
            .set_repo_roots(vec![fixture.root.join("repo")])
            .unwrap();
        fixture
    }

    fn write(&self, relative: &str, contents: &[u8]) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

fn document(name: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: A standard package.\n---\n\n{body}\n")
}

/// `openai-templates` ships a `reference.pptx` over four megabytes. The Skill is
/// three paragraphs of instructions telling the model to open it; losing the
/// whole package because one asset is large loses the instructions too.
#[test]
fn an_oversized_asset_does_not_cost_the_user_the_skill() {
    let fixture = Fixture::new();
    fixture.write(
        "repo/templates/SKILL.md",
        document("templates", "Use `assets/reference.pptx`.").as_bytes(),
    );
    fixture.write("repo/templates/assets/reference.pptx", &vec![0u8; 5 << 20]);

    let snapshot = fixture.service.list().unwrap();
    assert!(
        snapshot.issues.is_empty(),
        "a large asset is not a load failure: {:?}",
        snapshot.issues
    );
    let entry = snapshot.entry(&id("repo:templates")).expect("Skill loaded");
    let asset = entry
        .resources
        .iter()
        .find(|resource| resource.path == "assets/reference.pptx")
        .expect("the asset is still listed so the model knows it is there");
    assert_eq!(asset.size, 5 << 20);
}

/// A long `SKILL.md` is normal. The catalog bounds what one turn carries, and
/// that bound is this crate's alone — a consumer that re-derives its own and
/// rejects the result turns a good Skill into a failed conversation.
#[test]
fn a_long_document_is_bounded_for_the_turn_rather_than_refused() {
    let fixture = Fixture::new();
    let body = "Guidance about how to use this Skill in practice.\n".repeat(2_000);
    fixture.write("repo/long/SKILL.md", document("long", &body).as_bytes());

    let snapshot = fixture.service.list().unwrap();
    assert!(snapshot.issues.is_empty(), "{:?}", snapshot.issues);

    let catalog = fixture.service.turn_catalog().unwrap();
    let entry = catalog.entries.first().expect("the Skill is offered");
    assert!(entry.document.len() <= MAX_TURN_DOCUMENT_BYTES);
    // Head and tail both survive: the frontmatter routes, the closing notes
    // qualify.
    assert!(entry.document.starts_with("---"));
    assert!(entry.document.contains("in practice."));
}

/// Everything else a shipped package carries.
#[test]
fn the_ordinary_furniture_of_a_package_loads() {
    let fixture = Fixture::new();
    fixture.write(
        "repo/full/SKILL.md",
        document("full", "See `references/api.md`.").as_bytes(),
    );
    fixture.write("repo/full/LICENSE.txt", b"Apache-2.0");
    fixture.write("repo/full/README.md", b"# Full");
    fixture.write("repo/full/references/api.md", b"reference");
    fixture.write("repo/full/scripts/run.py", b"print('hi')");
    fixture.write("repo/full/assets/logo.svg", b"<svg/>");
    fixture.write(
        "repo/full/agents/openai.yaml",
        b"interface:\n  display_name: Full\n",
    );
    // A dot-directory left by whatever packaged it is not part of the Skill.
    fixture.write("repo/full/.git/config", b"[core]");

    let snapshot = fixture.service.list().unwrap();
    assert!(snapshot.issues.is_empty(), "{:?}", snapshot.issues);
    let entry = snapshot.entry(&id("repo:full")).expect("Skill loaded");
    assert_eq!(entry.display_name(), "Full");
    let paths = entry
        .resources
        .iter()
        .map(|resource| resource.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "LICENSE.txt",
            "README.md",
            "agents/openai.yaml",
            "assets/logo.svg",
            "references/api.md",
            "scripts/run.py",
        ]
    );
}

/// A directory of real packages, when one is pointed at. Skipped otherwise so
/// the suite stays hermetic.
#[test]
fn real_packages_load_completely() {
    let Ok(root) = std::env::var("HANAFUBUKI_SKILL_SMOKE_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    assert!(root.is_dir(), "{} is not a directory", root.display());
    let expected = count_skill_directories(&root);
    assert!(expected > 0, "{} holds no SKILL.md", root.display());

    let fixture = Fixture::over(vec![root.clone()]);
    let snapshot = fixture.service.list().unwrap();
    // A warning (a field ignored or shortened) still loads its Skill; only
    // an error costs one. The warnings are printed so a passing run still
    // shows what was left out.
    let (errors, warnings): (Vec<_>, Vec<_>) = snapshot
        .issues
        .iter()
        .partition(|issue| issue.severity == SkillIssueSeverity::Error);
    assert!(
        errors.is_empty(),
        "every package under {} must load: {errors:?} (warnings: {warnings:?})",
        root.display(),
    );
    for warning in &warnings {
        eprintln!("warning: {}: {}", warning.path, warning.message);
    }
    assert_eq!(snapshot.entries.len(), expected);

    let catalog = fixture.service.turn_catalog().unwrap();
    assert_eq!(catalog.entries.len(), expected);
    for entry in &catalog.entries {
        assert!(!entry.name.is_empty());
        assert!(!entry.document.is_empty());
        assert!(entry.document.len() <= MAX_TURN_DOCUMENT_BYTES);
        // Every listed resource is readable back by the path it was listed
        // under, which is what the model will pass to `skills_read`.
        for resource in &entry.resources {
            assert!(!resource.path.starts_with('/'));
            assert!(!resource.path.contains(".."));
        }
    }
}

fn count_skill_directories(directory: &Path) -> usize {
    if directory.join("SKILL.md").is_file() {
        return 1;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path().is_dir() && !entry.file_name().to_string_lossy().starts_with('.')
        })
        .map(|entry| count_skill_directories(&entry.path()))
        .sum()
}

fn id(value: &str) -> hanafubuki_agent_skills::SkillId {
    hanafubuki_agent_skills::SkillId::from_untrusted(value).unwrap()
}
