//! The path contract this crate owes the rest of the application.
//!
//! Everything this crate hands outward — every result field, every error
//! message, every approval detail — is read by a person or by a model, and both
//! will quote it back as the next request. A Windows verbatim path (`\\?\E:\x`)
//! is therefore not a cosmetic blemish: once one enters a transcript the model
//! keeps reusing it, and the reused string is not something a person would ever
//! have typed.
//!
//! These assertions run on every platform. On Linux `canonicalize` returns a
//! plain path and none of this can fail, so they are a guard rather than a
//! reproduction; the Windows-specific folding rules live in
//! `hanafubuki-agent-foundation`'s `PathKey` tests, which is the type this crate delegates
//! comparison to.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_approval::{
    ApprovalClock, ApprovalContext, ApprovalDecisionSource, ApprovalService, AuthorizedAction,
    PermissionActionDetails,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_workspace_fs::*;
use tempfile::TempDir;

/// Prefixes that must never reach a model, an interface, or a durable record.
const FORBIDDEN_PREFIXES: [&str; 2] = [r"\\?\", r"\\.\"];

#[derive(Default)]
struct TestClock;

impl ApprovalClock for TestClock {
    fn now_millis(&self) -> u128 {
        1
    }
}

fn policy(root: &Path) -> WorkspaceFsPolicy {
    WorkspaceFsPolicy::rooted_at(root.to_path_buf(), std::iter::empty::<std::path::PathBuf>())
        .unwrap()
}

fn approved(service: &WorkspaceFsService, request: &WorkspaceFsRequest) -> AuthorizedAction {
    let prepared = service.prepare(request).unwrap();
    let context =
        ApprovalContext::from_canonical_bytes("fs-call".to_string(), b"session-1").unwrap();
    ApprovalService::new(Arc::new(TestClock))
        .authorize(
            prepared.action().clone(),
            &context,
            Duration::from_secs(30),
            ApprovalDecisionSource::FullAccess,
        )
        .unwrap()
}

fn run(
    service: &WorkspaceFsService,
    request: &WorkspaceFsRequest,
) -> Result<WorkspaceFsResult, WorkspaceFsError> {
    let prepared = service.prepare(request)?;
    if prepared.action().kind == hanafubuki_agent_approval::ApprovalActionKind::FileRead {
        service.execute(request, None, &CancellationToken::default())
    } else {
        let approved = approved(service, request);
        service.execute(request, Some(&approved), &CancellationToken::default())
    }
}

fn assert_interchange(what: &str, text: &str) {
    for prefix in FORBIDDEN_PREFIXES {
        assert!(
            !text.contains(prefix),
            "{what} leaked a Windows verbatim path ({prefix}): {text}"
        );
    }
}

fn workspace() -> TempDir {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("tree/nested")).unwrap();
    fs::write(root.path().join("tree/a.txt"), "needle here\n").unwrap();
    fs::write(root.path().join("tree/nested/b.txt"), "needle too\n").unwrap();
    root
}

/// Every field of every result variant, checked as serialized JSON so a new
/// field cannot be added without also being covered.
#[test]
fn no_result_variant_exposes_a_verbatim_path() {
    let root = workspace();
    let service = WorkspaceFsService::new(policy(root.path()));

    let requests = [
        WorkspaceFsRequest::ListDirectory(ListDirectoryRequest {
            path: "tree".to_string(),
            max_depth: 4,
            max_entries: 50,
        }),
        WorkspaceFsRequest::SearchText(SearchTextRequest {
            path: "tree".to_string(),
            query: "needle".to_string(),
            max_depth: 4,
            max_results: 20,
            max_bytes: 4096,
        }),
        WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: "tree/a.txt".to_string(),
            offset: 0,
            max_bytes: 4096,
        }),
        WorkspaceFsRequest::WriteFile(WriteFileRequest {
            path: "written.txt".to_string(),
            content: "x\n".to_string(),
            overwrite: false,
        }),
        WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: "*** Begin Patch\n*** Add File: patched.txt\n+p\n*** End Patch".to_string(),
        }),
        WorkspaceFsRequest::MovePath(MovePathRequest {
            source: "written.txt".to_string(),
            destination: "moved.txt".to_string(),
        }),
        WorkspaceFsRequest::DeletePath(DeletePathRequest {
            path: "moved.txt".to_string(),
            recursive: false,
        }),
    ];

    // Serialized JSON escapes Windows separators (`\` becomes `\\`), so the
    // root must be compared in its escaped spelling.
    let root_in_json = serde_json::to_string(&root.path().to_string_lossy()).unwrap();
    let root_in_json = &root_in_json[1..root_in_json.len() - 1];

    for request in requests {
        let result = run(&service, &request).unwrap();
        let json = serde_json::to_string(&result).unwrap();
        assert_interchange("a filesystem result", &json);
        // A relative request must come back as an absolute location, or the
        // model cannot address it again.
        assert!(json.contains(root_in_json), "{json}");
    }
}

/// The approval card and the summary are what a person reads before granting a
/// mutation. They must name the location in the same spelling as the results.
#[test]
fn approval_summaries_and_details_expose_only_native_paths() {
    let root = workspace();
    let service = WorkspaceFsService::new(policy(root.path()));

    let prepared = service
        .prepare(&WorkspaceFsRequest::DeletePath(DeletePathRequest {
            path: "tree/a.txt".to_string(),
            recursive: false,
        }))
        .unwrap();

    assert_interchange("an approval summary", &prepared.action().summary);
    let Some(PermissionActionDetails::Paths { canonical_paths }) = &prepared.action().details
    else {
        panic!("expected path details, got {:?}", prepared.action().details);
    };
    for path in canonical_paths {
        assert_interchange("an approval path detail", path);
    }
}

/// Error text is the only feedback the model gets when a call fails. A verbatim
/// path there is worse than in a result: the model is being told to correct
/// something, using a spelling it cannot have produced.
#[test]
fn no_error_message_exposes_a_verbatim_path() {
    let root = TempDir::new().unwrap();
    let inside = root.path().join("workspace");
    fs::create_dir_all(&inside).unwrap();
    fs::write(root.path().join("outside.txt"), "original\n").unwrap();
    fs::write(inside.join("present.txt"), "original\n").unwrap();
    let service = WorkspaceFsService::new(policy(&inside));

    let failures = [
        // A preimage that does not match.
        WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: "*** Begin Patch\n*** Update File: present.txt\n@@\n-absent line\n+changed\n*** End Patch"
                .to_string(),
        }),
        // A target that is not there.
        WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: "absent.txt".to_string(),
            offset: 0,
            max_bytes: 1024,
        }),
        // A directory where a file was required.
        WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: ".".to_string(),
            offset: 0,
            max_bytes: 1024,
        }),
    ];

    for request in failures {
        let error = run(&service, &request).unwrap_err();
        assert_interchange("a filesystem error", &error.to_string());
    }
}

/// The reported Windows failure. The workspace base is verbatim there, Win32
/// does not normalize a verbatim path, and `<base>\..\sibling` simply fails to
/// open — so `..` had to be collapsed before the filesystem is touched.
#[test]
fn a_relative_parent_request_resolves_instead_of_failing() {
    let root = TempDir::new().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir_all(workspace.join("nested")).unwrap();
    fs::write(workspace.join("sibling.txt"), "content\n").unwrap();
    let service = WorkspaceFsService::new(policy(&workspace));

    let result = run(
        &service,
        &WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: "nested/../sibling.txt".to_string(),
            offset: 0,
            max_bytes: 1024,
        }),
    )
    .unwrap();

    let WorkspaceFsResult::FileRead(read) = result else {
        panic!("expected a file read");
    };
    assert_eq!(read.content, "content\n");
    assert!(read.path.ends_with("sibling.txt"), "{}", read.path);
    assert_interchange("a traversal read", &read.path);
}

/// A missing target and an unreadable one are different facts. Collapsing them
/// would make `resolve_target` retry a permission failure as a create and tell
/// the model the file does not exist.
#[test]
fn a_missing_path_is_reported_as_missing_not_as_a_resolution_failure() {
    let root = workspace();
    let service = WorkspaceFsService::new(policy(root.path()));

    let error = run(
        &service,
        &WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: "tree/absent.txt".to_string(),
            offset: 0,
            max_bytes: 1024,
        }),
    )
    .unwrap_err();

    assert!(
        matches!(error, WorkspaceFsError::PathMissing(_)),
        "expected a missing-path error, got {error:?}"
    );
}

/// A protected root and a request may be spelled differently and still be the
/// same directory. Containment is decided on identity, so the guard holds
/// whichever spelling each side happens to carry.
#[test]
fn protection_holds_across_spellings_of_one_location() {
    let root = TempDir::new().unwrap();
    let workspace = root.path().join("workspace");
    let protected = workspace.join("app-data");
    fs::create_dir_all(protected.join("state")).unwrap();
    fs::write(protected.join("state/secret.txt"), "private\n").unwrap();

    // The protected root is registered through a path that needs normalizing;
    // the request arrives in yet another shape.
    let policy =
        WorkspaceFsPolicy::rooted_at(&workspace, [workspace.join("nested/../app-data")]).unwrap();
    let service = WorkspaceFsService::new(policy);

    for spelling in [
        "app-data/state/secret.txt",
        "./app-data/state/secret.txt",
        "app-data/state/../state/secret.txt",
    ] {
        let error = run(
            &service,
            &WorkspaceFsRequest::ReadFile(ReadFileRequest {
                path: spelling.to_string(),
                offset: 0,
                max_bytes: 1024,
            }),
        )
        .unwrap_err();
        assert!(
            matches!(error, WorkspaceFsError::ProtectedPath(_)),
            "{spelling} was not protected: {error:?}"
        );
        assert_interchange("a protected path error", &error.to_string());
    }
}

/// One patch may touch each location once. The rule is about locations, not
/// about the strings naming them, so two spellings of one file must collide.
#[test]
fn one_patch_cannot_reach_the_same_file_under_two_spellings() {
    let root = TempDir::new().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir_all(workspace.join("nested")).unwrap();
    fs::write(workspace.join("target.txt"), "original\n").unwrap();
    let service = WorkspaceFsService::new(policy(&workspace));

    let error = run(
        &service,
        &WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: "*** Begin Patch\n\
                    *** Update File: target.txt\n\
                    @@\n-original\n+first\n\
                    *** Update File: nested/../target.txt\n\
                    @@\n-original\n+second\n\
                    *** End Patch"
                .to_string(),
        }),
    )
    .unwrap_err();

    assert!(
        matches!(&error, WorkspaceFsError::InvalidRequest(message) if message.contains("only once")),
        "{error:?}"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("target.txt")).unwrap(),
        "original\n"
    );
}

/// The case-folding half of the two-spellings rule. Windows filesystems are
/// case insensitive, so `target.txt` and `TARGET.txt` are one file, and one
/// patch must not touch it twice under the two spellings.
#[cfg(windows)]
#[test]
fn one_patch_cannot_reach_an_existing_file_under_two_case_spellings() {
    let root = TempDir::new().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("target.txt"), "original\n").unwrap();
    let service = WorkspaceFsService::new(policy(&workspace));

    let error = run(
        &service,
        &WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: "*** Begin Patch\n\
                    *** Update File: target.txt\n\
                    @@\n-original\n+first\n\
                    *** Update File: TARGET.txt\n\
                    @@\n-original\n+second\n\
                    *** End Patch"
                .to_string(),
        }),
    )
    .unwrap_err();

    assert!(
        matches!(&error, WorkspaceFsError::InvalidRequest(message) if message.contains("only once")),
        "{error:?}"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("target.txt")).unwrap(),
        "original\n"
    );
}

/// A brand-new file has no on-disk casing to canonicalize to, so the collision
/// must still fire through the missing-path key, which is built from the
/// requested name.
#[cfg(windows)]
#[test]
fn one_patch_cannot_reach_a_new_file_under_two_case_spellings() {
    let root = TempDir::new().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let service = WorkspaceFsService::new(policy(&workspace));

    let error = run(
        &service,
        &WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: "*** Begin Patch\n\
                    *** Add File: NEW.txt\n\
                    +first\n\
                    *** Add File: new.txt\n\
                    +second\n\
                    *** End Patch"
                .to_string(),
        }),
    )
    .unwrap_err();

    assert!(
        matches!(&error, WorkspaceFsError::InvalidRequest(message) if message.contains("only once")),
        "{error:?}"
    );
    assert!(
        !workspace.join("NEW.txt").exists() && !workspace.join("new.txt").exists(),
        "a rejected patch must create nothing"
    );
}

/// On a case-sensitive filesystem `target.txt` and `TARGET.txt` are two real
/// files and one patch may legitimately touch both. The Windows folding must not
/// leak onto it. Linux's filesystems are; macOS's default APFS is not, and there
/// both names resolve to one file, which the patch is rightly refused for.
#[cfg(not(any(windows, target_os = "macos")))]
#[test]
fn one_patch_may_touch_case_distinct_files_on_posix() {
    let root = TempDir::new().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("target.txt"), "original\n").unwrap();
    fs::write(workspace.join("TARGET.txt"), "original\n").unwrap();
    let service = WorkspaceFsService::new(policy(&workspace));

    let result = run(
        &service,
        &WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: "*** Begin Patch\n\
                    *** Update File: target.txt\n\
                    @@\n-original\n+first\n\
                    *** Update File: TARGET.txt\n\
                    @@\n-original\n+second\n\
                    *** End Patch"
                .to_string(),
        }),
    )
    .unwrap();
    assert!(matches!(result, WorkspaceFsResult::PatchApplied(_)));
    assert_eq!(
        fs::read_to_string(workspace.join("target.txt")).unwrap(),
        "first\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("TARGET.txt")).unwrap(),
        "second\n"
    );
}

/// The Agent works on the WebUI instances this application manages, and an
/// attached instance lives wherever the user put it — often on another volume.
/// A single root would make every edit there demand elevation, which is what
/// made the reported `E:\Softwares\ComfyUI` session unworkable.
#[test]
fn a_second_root_is_in_workspace_without_elevation() {
    let root = TempDir::new().unwrap();
    let managed = root.path().join("instances");
    let attached = root.path().join("elsewhere").join("ComfyUI");
    fs::create_dir_all(managed.join("managed-a")).unwrap();
    fs::create_dir_all(attached.join("custom_nodes")).unwrap();
    fs::write(attached.join("custom_nodes/node.py"), "original\n").unwrap();

    let service = WorkspaceFsService::new(
        WorkspaceFsPolicy::new(
            [managed.clone(), attached.clone()],
            std::iter::empty::<std::path::PathBuf>(),
        )
        .unwrap(),
    );

    // An absolute path under the second root patches as an ordinary edit.
    let patched = attached.join("custom_nodes/node.py");
    let result = run(
        &service,
        &WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: format!(
                "*** Begin Patch\n*** Update File: {}\n@@\n-original\n+rewritten\n*** End Patch",
                patched.display()
            ),
        }),
    )
    .unwrap();
    assert!(matches!(result, WorkspaceFsResult::PatchApplied(_)));
    assert_eq!(fs::read_to_string(&patched).unwrap(), "rewritten\n");

    // A path under neither root is classified as an elevated patch: the exact
    // external path is frozen into the approval action rather than silently
    // applied.
    fs::write(root.path().join("outside.txt"), "original\n").unwrap();
    let prepared = service
        .prepare(&WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: format!(
                "*** Begin Patch\n*** Update File: {}\n@@\n-original\n+changed\n*** End Patch",
                root.path().join("outside.txt").display()
            ),
        }))
        .unwrap();
    assert_eq!(
        prepared.action().risk,
        hanafubuki_agent_approval::ApprovalRiskClass::Critical
    );
    assert!(
        prepared
            .action()
            .summary
            .contains("writing outside the workspace"),
        "{:?}",
        prepared.action().summary
    );
    // Approved, it applies to the external path without leaking a verbatim
    // path into any result.
    let result = run(
        &service,
        &WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
            patch: format!(
                "*** Begin Patch\n*** Update File: {}\n@@\n-original\n+changed\n*** End Patch",
                root.path().join("outside.txt").display()
            ),
        }),
    )
    .unwrap();
    assert!(matches!(result, WorkspaceFsResult::PatchApplied(_)));
    assert_eq!(
        fs::read_to_string(root.path().join("outside.txt")).unwrap(),
        "changed\n"
    );
}

/// Relative requests need one unambiguous base, and it is the first root. A
/// model that means a different root has the absolute path — every path it is
/// handed back is absolute.
#[test]
fn relative_requests_resolve_against_the_first_root() {
    let root = TempDir::new().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("shared.txt"), "from first\n").unwrap();
    fs::write(second.join("shared.txt"), "from second\n").unwrap();

    let service = WorkspaceFsService::new(
        WorkspaceFsPolicy::new(
            [first.clone(), second],
            std::iter::empty::<std::path::PathBuf>(),
        )
        .unwrap(),
    );

    let WorkspaceFsResult::FileRead(read) = run(
        &service,
        &WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: "shared.txt".to_string(),
            offset: 0,
            max_bytes: 1024,
        }),
    )
    .unwrap() else {
        panic!("expected a file read");
    };
    assert_eq!(read.content, "from first\n");
}

/// A root nested inside another widens nothing, so the policy keeps only the
/// outer one and containment is unchanged.
#[test]
fn a_nested_root_does_not_change_containment() {
    let root = TempDir::new().unwrap();
    let outer = root.path().join("outer");
    let inner = outer.join("inner");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("file.txt"), "content\n").unwrap();

    let policy = WorkspaceFsPolicy::new(
        [outer.clone(), inner.clone()],
        std::iter::empty::<std::path::PathBuf>(),
    )
    .unwrap();
    assert_eq!(policy.roots().count(), 1);

    let service = WorkspaceFsService::new(policy);
    let WorkspaceFsResult::FileRead(read) = run(
        &service,
        &WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: "inner/file.txt".to_string(),
            offset: 0,
            max_bytes: 1024,
        }),
    )
    .unwrap() else {
        panic!("expected a file read");
    };
    assert_eq!(read.content, "content\n");
}

/// A policy with no root would treat every path as external, which is a
/// misconfiguration rather than a strict default.
#[test]
fn a_policy_needs_at_least_one_root() {
    assert!(WorkspaceFsPolicy::new(
        std::iter::empty::<std::path::PathBuf>(),
        std::iter::empty::<std::path::PathBuf>(),
    )
    .is_err());
}
