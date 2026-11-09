use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_approval::{
    ApprovalClock, ApprovalContext, ApprovalDecisionSource, ApprovalService, AuthorizedAction,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_workspace_fs::test_support::{
    service_with_partial_write_failure, service_with_publish_cancellation,
    service_with_publish_failures,
};
use hanafubuki_agent_workspace_fs::*;
use tempfile::TempDir;

#[derive(Default)]
struct TestClock;

impl ApprovalClock for TestClock {
    fn now_millis(&self) -> u128 {
        1
    }
}

fn policy(root: &Path) -> WorkspaceFsPolicy {
    WorkspaceFsPolicy::rooted_at(root, std::iter::empty::<std::path::PathBuf>()).unwrap()
}

fn approved(service: &WorkspaceFsService, request: &WorkspaceFsRequest) -> AuthorizedAction {
    let prepared = service.prepare(request).unwrap();
    let context =
        ApprovalContext::from_canonical_bytes("fs-call".to_string(), b"session-1").unwrap();
    let approvals = ApprovalService::new(Arc::new(TestClock));
    approvals
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

#[test]
fn reads_are_unreviewed_and_mutations_require_exact_authorization() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("tree")).unwrap();
    fs::write(temp.path().join("tree/a.txt"), "first needle\nsecond\n").unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));

    let listing = WorkspaceFsRequest::ListDirectory(ListDirectoryRequest {
        path: "tree".to_string(),
        max_depth: 2,
        max_entries: 20,
    });
    let WorkspaceFsResult::DirectoryListing(listing_result) = service
        .execute(&listing, None, &CancellationToken::default())
        .unwrap()
    else {
        panic!("listing result")
    };
    assert_eq!(listing_result.entries.len(), 1);

    let search = WorkspaceFsRequest::SearchText(SearchTextRequest {
        path: "tree".to_string(),
        query: "needle".to_string(),
        max_depth: 2,
        max_results: 10,
        max_bytes: 4096,
    });
    let WorkspaceFsResult::SearchResults(search_result) = run(&service, &search).unwrap() else {
        panic!("search result")
    };
    assert_eq!(search_result.matches.len(), 1);

    let read = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "tree/a.txt".to_string(),
        offset: 0,
        max_bytes: 5,
    });
    let WorkspaceFsResult::FileRead(read_result) = run(&service, &read).unwrap() else {
        panic!("read result")
    };
    assert_eq!(read_result.content, "first");
    assert!(read_result.truncated);

    let write = WorkspaceFsRequest::WriteFile(WriteFileRequest {
        path: "tree/new.txt".to_string(),
        content: "old value".to_string(),
        overwrite: false,
    });
    assert!(matches!(
        run(&service, &write).unwrap(),
        WorkspaceFsResult::FileWritten(_)
    ));
    let patch = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: "*** Begin Patch\n*** Update File: tree/new.txt\n@@\n-old value\n+new value\n*** End Patch".to_string(),
    });
    assert!(matches!(
        run(&service, &patch).unwrap(),
        WorkspaceFsResult::PatchApplied(_)
    ));

    let move_request = WorkspaceFsRequest::MovePath(MovePathRequest {
        source: "tree/new.txt".to_string(),
        destination: "tree/moved.txt".to_string(),
    });
    assert!(matches!(
        run(&service, &move_request).unwrap(),
        WorkspaceFsResult::PathMoved(_)
    ));
    let delete = WorkspaceFsRequest::DeletePath(DeletePathRequest {
        path: "tree/moved.txt".to_string(),
        recursive: false,
    });
    assert!(matches!(
        run(&service, &delete).unwrap(),
        WorkspaceFsResult::PathDeleted(_)
    ));
    assert!(!temp.path().join("tree/moved.txt").exists());
}

#[test]
fn read_file_pages_utf8_content_from_the_returned_next_offset() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("paged.txt"), "abc😀xyz").unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));

    let first = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "paged.txt".to_string(),
        offset: 0,
        max_bytes: 4,
    });
    let WorkspaceFsResult::FileRead(first) = run(&service, &first).unwrap() else {
        panic!("first page")
    };
    assert_eq!(first.content, "abc");
    assert_eq!(first.offset, 0);
    assert_eq!(first.next_offset, 3);
    assert!(first.truncated);

    let second = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "paged.txt".to_string(),
        offset: first.next_offset,
        max_bytes: 4,
    });
    let WorkspaceFsResult::FileRead(second) = run(&service, &second).unwrap() else {
        panic!("second page")
    };
    assert_eq!(second.content, "😀");
    assert_eq!(second.offset, 3);
    assert_eq!(second.next_offset, 7);
    assert!(second.truncated);

    let third = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "paged.txt".to_string(),
        offset: second.next_offset,
        max_bytes: 4,
    });
    let WorkspaceFsResult::FileRead(third) = run(&service, &third).unwrap() else {
        panic!("third page")
    };
    assert_eq!(third.content, "xyz");
    assert_eq!(third.offset, 7);
    assert_eq!(third.next_offset, 10);
    assert!(!third.truncated);
}

#[test]
fn read_file_rejects_offsets_past_eof_and_files_larger_than_128_mib() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("small.txt"), "small").unwrap();
    let exact = fs::File::create(temp.path().join("exact-limit.txt")).unwrap();
    exact.set_len(MAX_READ_FILE_BYTES).unwrap();
    let oversized = fs::File::create(temp.path().join("oversized.txt")).unwrap();
    oversized.set_len(MAX_READ_FILE_BYTES + 1).unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));

    let past_eof = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "small.txt".to_string(),
        offset: 6,
        max_bytes: MIN_READ_PAGE_BYTES,
    });
    assert!(matches!(
        service.prepare(&past_eof),
        Err(WorkspaceFsError::InvalidRequest(message)) if message.contains("exceeds file size")
    ));

    let exact_eof = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "exact-limit.txt".to_string(),
        offset: MAX_READ_FILE_BYTES as usize,
        max_bytes: MIN_READ_PAGE_BYTES,
    });
    let WorkspaceFsResult::FileRead(exact_eof) = run(&service, &exact_eof).unwrap() else {
        panic!("exact limit EOF page")
    };
    assert!(exact_eof.content.is_empty());
    assert_eq!(exact_eof.next_offset, MAX_READ_FILE_BYTES as usize);
    assert!(!exact_eof.truncated);

    let oversized = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "oversized.txt".to_string(),
        offset: 0,
        max_bytes: MAX_READ_PAGE_BYTES,
    });
    assert!(matches!(
        service.prepare(&oversized),
        Err(WorkspaceFsError::LimitExceeded(message))
            if message.contains("read file exceeds")
    ));
}

#[test]
fn search_text_uses_ripgrep_style_regexes_and_reports_each_match_column() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("matches.txt"),
        "prefix Alpha 123 and alpha 456\n",
    )
    .unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::SearchText(SearchTextRequest {
        path: ".".to_string(),
        query: r"(?i)alpha\s+\d+".to_string(),
        max_depth: 2,
        max_results: 10,
        max_bytes: 4096,
    });

    let WorkspaceFsResult::SearchResults(result) = run(&service, &request).unwrap() else {
        panic!("search result")
    };
    assert_eq!(result.matches.len(), 2);
    assert_eq!(result.matches[0].line, 1);
    assert_eq!(result.matches[0].column, 8);
    assert_eq!(result.matches[1].column, 22);
    assert_eq!(result.matches[0].preview, "prefix Alpha 123 and alpha 456");
    assert!(!result.truncated);
}

#[test]
fn search_text_rejects_invalid_regular_expressions_during_preparation() {
    let temp = TempDir::new().unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::SearchText(SearchTextRequest {
        path: ".".to_string(),
        query: "(".to_string(),
        max_depth: 2,
        max_results: 10,
        max_bytes: 4096,
    });

    assert!(matches!(
        service.prepare(&request),
        Err(WorkspaceFsError::InvalidRequest(message))
            if message.contains("invalid search regular expression")
    ));
}

#[test]
fn search_text_honors_ripgrep_ignore_hidden_and_binary_defaults() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join(".gitignore"), "gitignored.txt\n").unwrap();
    fs::write(temp.path().join(".ignore"), "ignored.txt\n").unwrap();
    fs::write(temp.path().join(".rgignore"), "rgignored.txt\n").unwrap();
    fs::write(temp.path().join("gitignored.txt"), "needle\n").unwrap();
    fs::write(temp.path().join("ignored.txt"), "needle\n").unwrap();
    fs::write(temp.path().join("rgignored.txt"), "needle\n").unwrap();
    fs::write(temp.path().join(".hidden.txt"), "needle\n").unwrap();
    fs::write(
        temp.path().join("binary.bin"),
        b"needle before nul\n\0binary",
    )
    .unwrap();
    fs::write(temp.path().join("visible.txt"), "needle\n").unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::SearchText(SearchTextRequest {
        path: ".".to_string(),
        query: "needle".to_string(),
        max_depth: 2,
        max_results: 10,
        max_bytes: 4096,
    });

    let WorkspaceFsResult::SearchResults(result) = run(&service, &request).unwrap() else {
        panic!("search result")
    };
    assert_eq!(result.matches.len(), 1);
    assert!(result.matches[0].path.ends_with("visible.txt"));
    assert_eq!(result.scanned_files, 2);
    assert!(!result.truncated);
}

#[test]
fn search_text_keeps_result_limits_typed_as_truncation() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("matches.txt"), "needle needle\n").unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::SearchText(SearchTextRequest {
        path: ".".to_string(),
        query: "needle".to_string(),
        max_depth: 2,
        max_results: 1,
        max_bytes: 4096,
    });

    let WorkspaceFsResult::SearchResults(result) = run(&service, &request).unwrap() else {
        panic!("search result")
    };
    assert_eq!(result.matches.len(), 1);
    assert!(result.truncated);
}

#[test]
fn search_text_does_not_publish_an_early_match_from_a_binary_file_at_the_result_limit() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("a-binary.bin"), b"needle\n\0binary").unwrap();
    fs::write(temp.path().join("z-visible.txt"), "needle\n").unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::SearchText(SearchTextRequest {
        path: ".".to_string(),
        query: "needle".to_string(),
        max_depth: 2,
        max_results: 1,
        max_bytes: 4096,
    });

    let WorkspaceFsResult::SearchResults(result) = run(&service, &request).unwrap() else {
        panic!("search result")
    };
    assert_eq!(result.matches.len(), 1);
    assert!(result.matches[0].path.ends_with("z-visible.txt"));
    assert!(result.truncated);
}

#[test]
fn bounded_binary_reads_use_the_callers_explicit_limit() {
    let temp = TempDir::new().unwrap();
    let bytes = vec![7u8; 4097];
    fs::write(temp.path().join("image.bin"), &bytes).unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));

    service
        .validate_bounded_binary_read("image.bin", bytes.len())
        .unwrap();
    let selected = service
        .read_bounded_binary("image.bin", bytes.len(), &CancellationToken::default())
        .unwrap();

    assert_eq!(selected.bytes, bytes);
    assert!(service
        .validate_bounded_binary_read("image.bin", 4096)
        .is_err());
}

#[cfg(unix)]
#[test]
fn symlink_target_replacement_invalidates_the_approved_action() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("a.txt"), "safe").unwrap();
    fs::write(temp.path().join("b.txt"), "different").unwrap();
    symlink("a.txt", temp.path().join("link.txt")).unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "link.txt".to_string(),
        offset: 0,
        max_bytes: 64,
    });
    let approval = approved(&service, &request);
    fs::remove_file(temp.path().join("link.txt")).unwrap();
    symlink("b.txt", temp.path().join("link.txt")).unwrap();
    assert!(matches!(
        service.execute(&request, Some(&approval), &CancellationToken::default()),
        Err(WorkspaceFsError::ApprovalMismatch)
    ));
}

#[test]
fn same_path_replacement_is_rejected_and_new_target_is_untouched() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("target.txt");
    fs::write(&path, "approved preimage").unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::DeletePath(DeletePathRequest {
        path: "target.txt".to_string(),
        recursive: false,
    });
    let approval = approved(&service, &request);
    fs::rename(&path, temp.path().join("old.txt")).unwrap();
    fs::write(&path, "replacement").unwrap();
    assert!(matches!(
        service.execute(&request, Some(&approval), &CancellationToken::default()),
        Err(WorkspaceFsError::ApprovalMismatch)
    ));
    assert_eq!(fs::read_to_string(path).unwrap(), "replacement");
}

#[test]
fn atomic_write_failure_and_mid_publish_cancellation_keep_the_preimage() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("value.txt");
    fs::write(&path, "original").unwrap();
    let request = WorkspaceFsRequest::WriteFile(WriteFileRequest {
        path: "value.txt".to_string(),
        content: "replacement".to_string(),
        overwrite: true,
    });

    let failing = service_with_publish_failures(policy(temp.path()), 1);
    let approval = approved(&failing, &request);
    assert!(matches!(
        failing.execute(&request, Some(&approval), &CancellationToken::default()),
        Err(WorkspaceFsError::Io(_))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");

    let partial_request = WorkspaceFsRequest::WriteFile(WriteFileRequest {
        path: "value.txt".to_string(),
        content: "x".repeat(128 * 1024),
        overwrite: true,
    });
    let partial = service_with_partial_write_failure(policy(temp.path()));
    let approval = approved(&partial, &partial_request);
    assert!(matches!(
        partial.execute(
            &partial_request,
            Some(&approval),
            &CancellationToken::default()
        ),
        Err(WorkspaceFsError::Io(_))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");

    let cancellation = CancellationToken::default();
    let cancelling = service_with_publish_cancellation(policy(temp.path()), cancellation.clone());
    let approval = approved(&cancelling, &request);
    assert!(matches!(
        cancelling.execute(&request, Some(&approval), &cancellation),
        Err(WorkspaceFsError::Cancelled)
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    let names = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(names
        .iter()
        .all(|name| !name.starts_with(".hanafubuki-agent-write-")));
}

#[test]
fn large_files_are_truncated_or_rejected_without_unbounded_reads() {
    let temp = TempDir::new().unwrap();
    let large = vec![b'x'; MAX_WRITE_BYTES + 1];
    fs::write(temp.path().join("large.txt"), &large).unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let read = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "large.txt".to_string(),
        offset: 0,
        max_bytes: 1024,
    });
    let WorkspaceFsResult::FileRead(result) = run(&service, &read).unwrap() else {
        panic!("read")
    };
    assert_eq!(result.bytes, 1024);
    assert!(result.truncated);

    let patch = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: "*** Begin Patch\n*** Update File: large.txt\n@@\n-x\n+y\n*** End Patch".to_string(),
    });
    assert!(matches!(
        run(&service, &patch),
        Err(WorkspaceFsError::LimitExceeded(_))
    ));
}

#[test]
fn codex_patch_applies_add_update_move_and_delete_under_one_approval() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("tree")).unwrap();
    fs::write(temp.path().join("tree/update.txt"), "heading\nold\n").unwrap();
    fs::write(temp.path().join("tree/move.txt"), "move me\n").unwrap();
    fs::write(temp.path().join("tree/delete.txt"), "obsolete\n").unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: "*** Begin Patch\n*** Add File: tree/added.txt\n+added\n*** Update File: tree/update.txt\n@@ heading\n-old\n+new\n*** Update File: tree/move.txt\n*** Move to: tree/moved.txt\n@@\n-move me\n+moved\n*** Delete File: tree/delete.txt\n*** End Patch".to_string(),
    });

    let WorkspaceFsResult::PatchApplied(result) = run(&service, &request).unwrap() else {
        panic!("patch result")
    };
    assert_eq!(result.added.len(), 1);
    assert_eq!(result.modified.len(), 2);
    assert_eq!(result.deleted.len(), 1);
    assert_eq!(
        fs::read_to_string(temp.path().join("tree/update.txt")).unwrap(),
        "heading\nnew\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("tree/moved.txt")).unwrap(),
        "moved\n"
    );
    assert!(!temp.path().join("tree/move.txt").exists());
    assert!(!temp.path().join("tree/delete.txt").exists());
}

#[test]
fn patch_preimage_failure_happens_before_any_file_is_written() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("one.txt"), "one\n").unwrap();
    fs::write(temp.path().join("two.txt"), "two\n").unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));
    let request = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: "*** Begin Patch\n*** Update File: one.txt\n@@\n-one\n+changed\n*** Update File: two.txt\n@@\n-missing\n+changed\n*** End Patch".to_string(),
    });

    assert!(matches!(
        service.prepare(&request),
        Err(WorkspaceFsError::PreimageMismatch(_))
    ));
    assert_eq!(
        fs::read_to_string(temp.path().join("one.txt")).unwrap(),
        "one\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("two.txt")).unwrap(),
        "two\n"
    );
}

#[test]
fn protected_application_paths_are_rejected_before_approval() {
    let temp = TempDir::new().unwrap();
    let protected = temp.path().join("app-data");
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("credentials.json"), "secret").unwrap();
    let policy = WorkspaceFsPolicy::rooted_at(temp.path(), [protected]).unwrap();
    let service = WorkspaceFsService::new(policy);
    let request = WorkspaceFsRequest::ReadFile(ReadFileRequest {
        path: "app-data/credentials.json".to_string(),
        offset: 0,
        max_bytes: 1024,
    });
    assert!(matches!(
        service.prepare(&request),
        Err(WorkspaceFsError::ProtectedPath(_))
    ));
}

/// `apply_patch` is the only file-writing tool, so every shape the removed
/// `write_file` tool covered has to reach disk through a patch — including the
/// empty file, which is what an `Add File` hunk with no `+` lines means.
#[test]
fn apply_patch_creates_files_including_an_empty_one() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("pkg")).unwrap();
    let service = WorkspaceFsService::new(policy(temp.path()));

    let create = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: concat!(
            "*** Begin Patch\n",
            "*** Add File: pkg/__init__.py\n",
            "*** Add File: pkg/main.py\n",
            "+print(\"run\")\n",
            "*** End Patch"
        )
        .to_string(),
    });
    let WorkspaceFsResult::PatchApplied(result) = run(&service, &create).unwrap() else {
        panic!("patch applied")
    };
    assert_eq!(result.added.len(), 2);

    let empty = temp.path().join("pkg/__init__.py");
    assert!(empty.exists());
    assert_eq!(fs::read(&empty).unwrap(), Vec::<u8>::new());
    assert_eq!(
        fs::read_to_string(temp.path().join("pkg/main.py")).unwrap(),
        "print(\"run\")\n"
    );

    // Add File over an existing path replaces it, which is what the removed
    // `write_file(overwrite: true)` did.
    let replace = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: "*** Begin Patch\n*** Add File: pkg/main.py\n+print(\"replaced\")\n*** End Patch"
            .to_string(),
    });
    assert!(matches!(
        run(&service, &replace).unwrap(),
        WorkspaceFsResult::PatchApplied(_)
    ));
    assert_eq!(
        fs::read_to_string(temp.path().join("pkg/main.py")).unwrap(),
        "print(\"replaced\")\n"
    );

    // And an empty file can be emptied again rather than only created.
    fs::write(temp.path().join("pkg/notes.txt"), "stale").unwrap();
    let truncate = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: "*** Begin Patch\n*** Add File: pkg/notes.txt\n*** End Patch".to_string(),
    });
    assert!(matches!(
        run(&service, &truncate).unwrap(),
        WorkspaceFsResult::PatchApplied(_)
    ));
    assert_eq!(
        fs::read(temp.path().join("pkg/notes.txt")).unwrap(),
        Vec::<u8>::new()
    );
}

/// Writing outside the workspace is a larger request than editing inside it:
/// the resolved external paths are frozen into the approval action so the
/// approver is asked for exactly that, and an authorized patch applies. There
/// is no envelope directive to declare elevation; approval decides.
#[test]
fn writing_outside_the_workspace_is_approved_as_an_elevated_patch() {
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let service = WorkspaceFsService::new(policy(workspace.path()));
    let target = outside.path().join("external.txt");

    let request = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: format!(
            "*** Begin Patch\n*** Add File: {}\n+outside\n*** End Patch",
            target.display()
        ),
    });
    // The prepared action names both spellings: the model can only correct the
    // header it wrote, and the approver needs to know where that text landed.
    let prepared = service.prepare(&request).unwrap();
    let action = prepared.action();
    assert_eq!(
        action.kind,
        hanafubuki_agent_approval::ApprovalActionKind::FilePatch
    );
    assert_eq!(
        action.risk,
        hanafubuki_agent_approval::ApprovalRiskClass::Critical
    );
    assert!(
        action.summary.contains("writing outside the workspace"),
        "{:?}",
        action.summary
    );

    // Approved as an ordinary mutation, the patch applies outside the workspace.
    assert!(matches!(
        run(&service, &request).unwrap(),
        WorkspaceFsResult::PatchApplied(_)
    ));
    assert_eq!(fs::read_to_string(&target).unwrap(), "outside\n");
}

/// An ordinary in-workspace patch needs nothing extra and is classified at the
/// ordinary patch risk rather than the elevated one.
#[test]
fn an_in_workspace_patch_stays_at_ordinary_patch_risk() {
    let workspace = TempDir::new().unwrap();
    let service = WorkspaceFsService::new(policy(workspace.path()));

    let plain = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: "*** Begin Patch\n*** Add File: inside.txt\n+a\n*** End Patch".to_string(),
    });
    let prepared = service.prepare(&plain).unwrap();
    assert_eq!(
        prepared.action().risk,
        hanafubuki_agent_approval::ApprovalRiskClass::High
    );
    assert!(
        !prepared
            .action()
            .summary
            .contains("writing outside the workspace"),
        "{:?}",
        prepared.action().summary
    );
    assert!(matches!(
        run(&service, &plain).unwrap(),
        WorkspaceFsResult::PatchApplied(_)
    ));
    assert_eq!(
        fs::read_to_string(workspace.path().join("inside.txt")).unwrap(),
        "a\n"
    );
}

/// The external-path classification runs on resolved paths, so `..` cannot walk
/// out of the workspace while looking like an ordinary relative edit.
#[test]
fn a_traversal_path_cannot_leave_the_workspace_as_an_ordinary_edit() {
    let root = TempDir::new().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(root.path().join("sibling.txt"), "original").unwrap();
    let service = WorkspaceFsService::new(policy(&workspace));

    let traversal = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: "*** Begin Patch\n*** Update File: ../sibling.txt\n@@\n-original\n+rewritten\n*** End Patch"
            .to_string(),
    });
    let prepared = service.prepare(&traversal).unwrap();
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
    assert!(matches!(
        run(&service, &traversal).unwrap(),
        WorkspaceFsResult::PatchApplied(_)
    ));
    assert_eq!(
        fs::read_to_string(root.path().join("sibling.txt")).unwrap(),
        "rewritten\n"
    );
}
