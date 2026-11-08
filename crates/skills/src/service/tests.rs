use super::*;

use crate::model::{
    SkillCatalogListRequest, SkillCreateRequest, SkillDeleteFileRequest, SkillDuplicateRequest,
    SkillInstallRequest, SkillScope, SkillWriteFileRequest,
};

const DOCUMENT: &str = "---\nname: demo\ndescription: Does a thing.\n---\nBody.\n";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: std::path::PathBuf,
    service: SkillService,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_path_buf();
        let service = SkillService::new(SkillRoots {
            system: root.join("system"),
            user: root.join("user"),
            install_bundled_system_skills: false,
            state_file: root.join("state/skills.json"),
        })
        .unwrap();
        service.set_repo_roots(vec![root.join("repo")]).unwrap();
        Self {
            _temporary: temporary,
            root,
            service,
        }
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn reload(&self) -> crate::model::SkillCatalogSnapshot {
        self.service
            .list_with_request(SkillCatalogListRequest { force_reload: true })
            .unwrap()
    }
}

fn id(value: &str) -> SkillId {
    SkillId::from_untrusted(value).unwrap()
}

#[test]
fn every_root_is_discovered_and_scoped() {
    let fixture = Fixture::new();
    fixture.write("repo/lint/SKILL.md", DOCUMENT);
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture.write("system/builtin/SKILL.md", DOCUMENT);

    let snapshot = fixture.reload();
    let mut ids = snapshot
        .entries
        .iter()
        .map(|entry| entry.id.as_str().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(ids, vec!["repo:lint", "system:builtin", "user:pdf"]);
    assert!(snapshot.entries.iter().all(|entry| entry.enabled));
    assert_eq!(
        snapshot.entry(&id("user:pdf")).map(|entry| entry.editable),
        Some(true)
    );
    assert_eq!(
        snapshot.entry(&id("repo:lint")).map(|entry| entry.editable),
        Some(false)
    );
}

#[test]
fn the_repository_roots_follow_the_workspace_without_a_force_reload() {
    let fixture = Fixture::new();
    fixture.write("other/lint/SKILL.md", DOCUMENT);
    assert!(fixture
        .service
        .list()
        .unwrap()
        .entry(&id("repo:lint"))
        .is_none());

    // Same relative layout under the new root: only the changed set, not
    // the roots' stamps, can tell the cached scan is stale.
    fixture
        .service
        .set_repo_roots(vec![fixture.root.join("other")])
        .unwrap();
    let turn = fixture.service.turn_catalog().unwrap();
    let entry = turn
        .entries
        .iter()
        .find(|entry| entry.id == id("repo:lint"))
        .unwrap();
    assert_eq!(
        entry.directory,
        hanafubuki_agent_foundation::paths::ResolvedPath::resolve(&fixture.root.join("other/lint"))
            .unwrap()
            .native_text()
    );

    fixture.service.set_repo_roots(Vec::new()).unwrap();
    assert!(fixture
        .service
        .list()
        .unwrap()
        .entry(&id("repo:lint"))
        .is_none());
}

#[test]
fn a_discovered_skill_is_on_until_the_user_turns_it_off() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    let target = id("user:pdf");
    assert!(fixture.reload().entry(&target).unwrap().enabled);

    let after = fixture.service.disable(&target).unwrap();
    assert!(!after.entry(&target).unwrap().enabled);
    assert!(fixture.service.turn_catalog().unwrap().entries.is_empty());

    let after = fixture.service.enable(&target).unwrap();
    assert!(after.entry(&target).unwrap().enabled);
    assert_eq!(fixture.service.turn_catalog().unwrap().entries.len(), 1);
}

#[test]
fn an_unknown_skill_cannot_be_turned_off() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture.reload();
    assert!(fixture.service.disable(&id("user:absent")).is_err());
    assert!(fixture.service.disable(&id("user:pdf")).is_ok());
}

#[test]
fn installing_copies_a_directory_into_the_managed_root() {
    let fixture = Fixture::new();
    fixture.write("source/pdf-tools/SKILL.md", DOCUMENT);
    fixture.write("source/pdf-tools/references/api.md", "reference");
    fixture.write("source/pdf-tools/LICENSE.txt", "MIT");

    let entry = fixture
        .service
        .install(SkillInstallRequest {
            source_directory: fixture.root.join("source/pdf-tools"),
            overwrite: false,
        })
        .unwrap();
    assert_eq!(entry.id.as_str(), "user:pdf-tools");
    assert!(entry.enabled);
    assert!(fixture
        .root
        .join("user/pdf-tools/references/api.md")
        .is_file());

    // A second install of the same folder is refused unless asked to replace.
    assert!(fixture
        .service
        .install(SkillInstallRequest {
            source_directory: fixture.root.join("source/pdf-tools"),
            overwrite: false,
        })
        .is_err());
    assert!(fixture
        .service
        .install(SkillInstallRequest {
            source_directory: fixture.root.join("source/pdf-tools"),
            overwrite: true,
        })
        .is_ok());
}

#[test]
fn installing_accepts_a_wrapper_that_holds_one_skill() {
    // A cloned repository or an unpacked archive is usually a wrapper. Making
    // the user find the inner folder achieves nothing.
    let fixture = Fixture::new();
    fixture.write("clone/pdf-tools/SKILL.md", DOCUMENT);

    let entry = fixture
        .service
        .install(SkillInstallRequest {
            source_directory: fixture.root.join("clone"),
            overwrite: false,
        })
        .unwrap();
    assert_eq!(entry.id.as_str(), "user:pdf-tools");
}

#[test]
fn a_source_that_is_not_a_skill_is_refused_before_anything_is_written() {
    let fixture = Fixture::new();
    fixture.write("source/notes.txt", "nothing here");
    assert!(fixture
        .service
        .install(SkillInstallRequest {
            source_directory: fixture.root.join("source"),
            overwrite: false,
        })
        .is_err());
    assert!(!fixture.root.join("user/source").exists());
}

#[test]
fn inspecting_a_source_reports_what_it_would_install() {
    let fixture = Fixture::new();
    fixture.write("source/pdf-tools/SKILL.md", DOCUMENT);
    let inspection = fixture
        .service
        .inspect_source(&fixture.root.join("source/pdf-tools"))
        .unwrap();
    assert_eq!(inspection.folder_name, "pdf-tools");
    assert_eq!(inspection.name, "demo");
    assert!(!inspection.conflicts);
}

#[test]
fn creating_a_skill_writes_a_document_that_loads() {
    let fixture = Fixture::new();
    let entry = fixture
        .service
        .create(SkillCreateRequest {
            name: "PDF Tools".to_string(),
            description: "Work with PDF files.".to_string(),
        })
        .unwrap();
    assert_eq!(entry.id.as_str(), "user:pdf-tools");
    assert_eq!(entry.name, "PDF Tools");
    assert_eq!(entry.description, "Work with PDF files.");

    let read = fixture.service.read(&entry.id).unwrap();
    assert!(read.instructions.starts_with("---"));
    assert!(fixture
        .service
        .create(SkillCreateRequest {
            name: "pdf tools".to_string(),
            description: "Another.".to_string(),
        })
        .is_err());
}

#[test]
fn duplicating_copies_a_read_only_skill_into_the_managed_root() {
    let fixture = Fixture::new();
    fixture.write("system/builtin/SKILL.md", DOCUMENT);
    fixture.write("system/builtin/references/api.md", "reference");
    fixture.reload();

    let entry = fixture
        .service
        .duplicate(SkillDuplicateRequest {
            source: id("system:builtin"),
            name: "My Builtin".to_string(),
        })
        .unwrap();
    assert_eq!(entry.id.as_str(), "user:my-builtin");
    assert_eq!(entry.name, "My Builtin");
    assert!(entry.editable);
    assert!(fixture
        .root
        .join("user/my-builtin/references/api.md")
        .is_file());
    // The original is untouched.
    assert_eq!(
        fixture
            .service
            .read(&id("system:builtin"))
            .unwrap()
            .entry
            .name,
        "demo"
    );
}

#[test]
fn only_a_managed_skill_can_be_written_or_deleted() {
    let fixture = Fixture::new();
    fixture.write("system/builtin/SKILL.md", DOCUMENT);
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture.reload();

    assert!(fixture
        .service
        .write_file(SkillWriteFileRequest {
            id: id("system:builtin"),
            path: "references/new.md".to_string(),
            content: b"no".to_vec(),
        })
        .is_err());
    assert!(fixture.service.delete(&id("system:builtin")).is_err());

    let entry = fixture
        .service
        .write_file(SkillWriteFileRequest {
            id: id("user:pdf"),
            path: "references/new.md".to_string(),
            content: b"reference".to_vec(),
        })
        .unwrap();
    assert!(entry
        .resources
        .iter()
        .any(|resource| resource.path == "references/new.md"));

    let entry = fixture
        .service
        .delete_file(SkillDeleteFileRequest {
            id: id("user:pdf"),
            path: "references/new.md".to_string(),
        })
        .unwrap();
    assert!(entry.resources.is_empty());

    // SKILL.md is what makes the directory a Skill; removing it means removing
    // the Skill.
    assert!(fixture
        .service
        .delete_file(SkillDeleteFileRequest {
            id: id("user:pdf"),
            path: "SKILL.md".to_string(),
        })
        .is_err());

    assert!(fixture.service.delete(&id("user:pdf")).is_ok());
    assert!(fixture.reload().entry(&id("user:pdf")).is_none());
}

#[test]
fn a_write_cannot_escape_its_skill() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture.reload();
    for path in ["../escape.md", "/etc/passwd", ".hidden"] {
        assert!(
            fixture
                .service
                .write_file(SkillWriteFileRequest {
                    id: id("user:pdf"),
                    path: path.to_string(),
                    content: b"no".to_vec(),
                })
                .is_err(),
            "{path} should be refused"
        );
    }
}

#[test]
fn reads_serve_documents_and_resources_but_never_scripts() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture.write("user/pdf/references/api.md", "reference");
    fixture.write("user/pdf/scripts/run.py", "print()");
    fixture.reload();
    let target = id("user:pdf");

    assert!(fixture
        .service
        .read(&target)
        .unwrap()
        .instructions
        .contains("Body."));
    assert_eq!(
        fixture
            .service
            .read_enabled_resource(&target, "references/api.md")
            .unwrap()
            .content,
        "reference"
    );
    // A script is never disclosed as text.
    assert!(fixture
        .service
        .read_enabled_resource(&target, "scripts/run.py")
        .is_err());
    assert!(fixture.service.read_file(&target, "SKILL.md").is_ok());
    assert!(fixture
        .service
        .read_enabled_resource(&target, "references/absent.md")
        .is_err());
}

#[test]
fn a_disabled_skill_serves_authoring_reads_but_not_model_reads() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture.write("user/pdf/references/api.md", "reference");
    fixture.reload();
    let target = id("user:pdf");
    fixture.service.disable(&target).unwrap();

    assert!(fixture.service.read_file(&target, "SKILL.md").is_ok());
    assert!(fixture
        .service
        .read_enabled_resource(&target, "references/api.md")
        .is_err());
    assert!(fixture.service.turn_catalog().unwrap().entries.is_empty());
}

#[test]
fn the_turn_catalog_carries_the_whole_document() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture.write("user/pdf/references/api.md", "reference");
    fixture.reload();

    let catalog = fixture.service.turn_catalog().unwrap();
    assert_eq!(catalog.entries.len(), 1);
    let entry = &catalog.entries[0];
    assert_eq!(entry.id.as_str(), "user:pdf");
    assert!(entry.document.contains("Body."));
    assert_eq!(entry.resources.len(), 1);
}

#[test]
fn a_broken_skill_is_reported_beside_a_catalog_that_still_works() {
    let fixture = Fixture::new();
    fixture.write("user/good/SKILL.md", DOCUMENT);
    fixture.write("user/broken/SKILL.md", "not a skill\n");

    let snapshot = fixture.reload();
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.issues.len(), 1);
}

#[test]
fn a_watcher_diagnostic_rides_along_with_the_catalog() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture
        .service
        .set_watcher_diagnostic(Some("watching is unavailable".to_string()));
    let snapshot = fixture.service.list().unwrap();
    assert!(snapshot.issues.iter().any(|issue| issue.severity
        == crate::model::SkillIssueSeverity::Warning
        && issue.message.contains("watching is unavailable")));

    fixture.service.set_watcher_diagnostic(None);
    assert!(fixture.service.list().unwrap().issues.is_empty());
}

#[test]
fn the_scope_of_a_skill_survives_a_reload() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    let entry = fixture
        .reload()
        .entry(&id("user:pdf"))
        .cloned()
        .expect("the Skill loads");
    assert_eq!(entry.scope, SkillScope::User);
    assert_eq!(entry.id.scope(), SkillScope::User);
}

#[cfg(unix)]
#[test]
fn a_read_cannot_escape_its_skill_by_traversal_or_symlink() {
    let fixture = Fixture::new();
    fixture.write("user/pdf/SKILL.md", DOCUMENT);
    fixture.write("user/other/SKILL.md", DOCUMENT);
    fixture.write("outside/secret.md", "secret");
    std::fs::create_dir_all(fixture.root.join("user/pdf/references")).unwrap();
    std::os::unix::fs::symlink(
        fixture.root.join("outside/secret.md"),
        fixture.root.join("user/pdf/references/secret.md"),
    )
    .unwrap();
    fixture.reload();
    let target = id("user:pdf");

    for path in [
        "references/secret.md",
        "../other/SKILL.md",
        "../../outside/secret.md",
        "/etc/passwd",
    ] {
        assert!(
            fixture
                .service
                .read_enabled_resource(&target, path)
                .is_err(),
            "{path} should be refused"
        );
        assert!(
            fixture.service.read_file(&target, path).is_err(),
            "{path} should be refused"
        );
    }
}
