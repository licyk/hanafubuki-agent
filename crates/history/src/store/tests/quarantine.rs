//! What `open` cannot admit costs only its own session: the store opens, the
//! other sessions serve, recovery lists the refused one, and nothing of it is
//! deleted or rewritten. Integrity checks still reject the session.

use super::attachments::pixel_png;
use super::*;
use hanafubuki_agent_kernel::{AgentHistoryStoreError, AgentImageAttachmentRef};

pub(super) fn create_with_text(
    store: &FileAgentHistoryStore,
    session_id: AgentSessionId,
    text: &str,
) {
    let mut first = item(1, 0, text);
    first.source_session_id = session_id;
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem { item: first }],
        })
        .unwrap();
}

/// Stages an image for `session_id` and commits an item that references it.
fn commit_image(
    store: &FileAgentHistoryStore,
    session_id: AgentSessionId,
) -> AgentImageAttachmentRef {
    let attachment = store
        .stage_image_attachment(
            session_id,
            hanafubuki_agent_kernel::PreparedAgentImage {
                bytes: pixel_png(),
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "pixel.png".to_string(),
                detail: hanafubuki_agent_kernel::AgentImageDetail::Auto,
            },
        )
        .unwrap();
    let mut referencing = item(2, 0, "");
    referencing.source_session_id = session_id;
    referencing.content = ConversationItem::UserMessage {
        text: "look".to_string(),
        images: vec![attachment.clone()],
    };
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(session_id, 1, 3),
            history_mutations: vec![AgentHistoryMutation::AppendItem { item: referencing }],
        })
        .unwrap();
    attachment
}

pub(super) fn blob_path(
    paths: &AgentHistoryPaths,
    attachment: &AgentImageAttachmentRef,
) -> std::path::PathBuf {
    paths
        .attachments
        .join(attachment.source_session_id.value().to_string())
        .join("blobs")
        .join(format!("{}.blob", attachment.attachment_id.as_str()))
}

pub(super) fn close(
    store: Arc<FileAgentHistoryStore>,
    workers: BackgroundWorkerGroup,
    sessions: &[AgentSessionId],
) {
    for session_id in sessions {
        store.shutdown_session(*session_id).unwrap();
    }
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);
}

pub(super) fn reopen(
    paths: &AgentHistoryPaths,
) -> (BackgroundWorkerGroup, Arc<FileAgentHistoryStore>) {
    let workers = BackgroundWorkerGroup::default();
    let store = FileAgentHistoryStore::open(paths.clone(), workers.clone())
        .unwrap_or_else(|error| panic!("one session stopped the store: {error}"));
    (workers, Arc::new(store))
}

pub(super) fn rollout_of(
    paths: &AgentHistoryPaths,
    session_id: AgentSessionId,
) -> std::path::PathBuf {
    let mut files = Vec::new();
    collect_jsonl_files(&paths.root, &mut files).unwrap();
    files
        .into_iter()
        .find(|path| {
            path.file_stem()
                .unwrap()
                .to_string_lossy()
                .ends_with(&format!("-{session_id}"))
        })
        .unwrap()
}

/// The sessions recovery is told it cannot load, with their reasons.
pub(super) fn refused(store: &FileAgentHistoryStore) -> Vec<(AgentSessionId, String)> {
    let mut refused = store
        .load_recovery_candidates()
        .unwrap()
        .into_iter()
        .map(|candidate| {
            let failure = candidate.expect_err("no session here needs recovery");
            (failure.session_id, failure.error.to_string())
        })
        .collect::<Vec<_>>();
    refused.sort();
    refused
}

fn transcript_refused(store: &FileAgentHistoryStore, session_id: AgentSessionId) -> bool {
    matches!(
        store.list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 10,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        }),
        Err(AgentHistoryStoreError::ProjectionUnavailable(_))
    )
}

pub(super) fn serves(store: &FileAgentHistoryStore, session_id: AgentSessionId, text: &str) {
    let context = store.load_latest_model_context(session_id).unwrap();
    assert!(matches!(
        &context[0].content,
        ConversationItem::UserMessage { text: found, .. } if found == text
    ));
    assert!(!transcript_refused(store, session_id));
}

/// Files under each `.recovery` directory of `root`.
pub(super) fn quarantined_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if directory
                .file_name()
                .is_some_and(|name| name == ".recovery")
            {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn a_rollout_of_another_format_is_moved_aside_with_its_attachments_and_not_read_again() {
    let (_temporary, paths, workers, store) = open_store();
    let incompatible = store.allocate_session_id().unwrap();
    let healthy = store.allocate_session_id().unwrap();
    create_with_text(&store, incompatible, "old");
    let attachment = commit_image(&store, incompatible);
    create_with_text(&store, healthy, "healthy");
    close(store, workers, &[incompatible, healthy]);

    let path = rollout_of(&paths, incompatible);
    let rewritten = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| {
            let mut record: serde_json::Value = serde_json::from_str(line).unwrap();
            record["formatVersion"] =
                serde_json::json!(hanafubuki_agent_kernel::AGENT_HISTORY_FORMAT_VERSION + 1);
            format!("{}\n", serde_json::to_string(&record).unwrap())
        })
        .collect::<String>();
    fs::write(&path, &rewritten).unwrap();
    fs::remove_file(&paths.sqlite_database).unwrap();

    let (workers, rebuilt) = reopen(&paths);
    assert!(!path.exists());
    let moved = quarantined_files(&paths.active_sessions);
    assert_eq!(moved.len(), 1);
    assert_eq!(
        moved[0].parent().unwrap(),
        path.parent().unwrap().join(".recovery")
    );
    assert_eq!(moved[0].extension().unwrap(), "invalid");
    assert_eq!(fs::read_to_string(&moved[0]).unwrap(), rewritten);
    let moved_blob = paths
        .attachments
        .join(".recovery")
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .next()
        .unwrap()
        .join("blobs")
        .join(format!("{}.blob", attachment.attachment_id.as_str()));
    assert_eq!(fs::read(&moved_blob).unwrap(), pixel_png());
    assert!(!blob_path(&paths, &attachment).exists());
    let refused_sessions = refused(&rebuilt);
    assert_eq!(refused_sessions.len(), 1);
    assert_eq!(refused_sessions[0].0, incompatible);
    assert!(refused_sessions[0]
        .1
        .contains(&moved[0].display().to_string()));
    assert!(matches!(
        rebuilt.load_latest_model_context(incompatible),
        Err(AgentHistoryStoreError::ProjectionUnavailable(_))
    ));
    serves(&rebuilt, healthy, "healthy");
    assert!(rebuilt.allocate_session_id().unwrap().value() > healthy.value());
    close(rebuilt, workers, &[]);

    // The moved file lies outside every scan: a second rebuild neither reads
    // it nor moves it again, and the session no longer exists.
    fs::remove_file(&paths.sqlite_database).unwrap();
    let (workers, again) = reopen(&paths);
    assert_eq!(quarantined_files(&paths.active_sessions), moved);
    assert!(refused(&again).is_empty());
    assert!(matches!(
        again.load_latest_model_context(incompatible),
        Err(AgentHistoryStoreError::NotFound { .. })
    ));
    serves(&again, healthy, "healthy");
    close(again, workers, &[]);
}

#[test]
fn a_missing_attachment_blob_quarantines_only_its_session() {
    let (_temporary, paths, workers, store) = open_store();
    let broken = store.allocate_session_id().unwrap();
    let intact = store.allocate_session_id().unwrap();
    create_with_text(&store, broken, "broken");
    let missing = commit_image(&store, broken);
    create_with_text(&store, intact, "intact");
    let kept = commit_image(&store, intact);
    close(store, workers, &[broken, intact]);
    fs::remove_file(blob_path(&paths, &missing)).unwrap();

    // With the index kept, and with it rebuilt from the rollouts.
    for rebuild in [false, true] {
        if rebuild {
            fs::remove_file(&paths.sqlite_database).unwrap();
        }
        let (workers, reopened) = reopen(&paths);
        let refused_sessions = refused(&reopened);
        assert_eq!(
            refused_sessions
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            vec![broken]
        );
        assert!(refused_sessions[0].1.contains("could not be read"));
        assert!(transcript_refused(&reopened, broken));
        assert!(reopened.load_runtime_state(broken).is_err());
        serves(&reopened, intact, "intact");
        assert_eq!(reopened.load_image_attachment(&kept).unwrap(), pixel_png());
        assert!(rollout_of(&paths, broken).is_file());
        close(reopened, workers, &[]);
    }
}

#[test]
fn an_undecodable_item_quarantines_only_its_session_and_is_not_rewritten() {
    let (_temporary, paths, workers, store) = open_store();
    let broken = store.allocate_session_id().unwrap();
    let intact = store.allocate_session_id().unwrap();
    create_with_text(&store, broken, "broken");
    create_with_text(&store, intact, "intact");
    let kept = commit_image(&store, intact);
    close(store, workers, &[broken, intact]);
    let broken_id = i64::try_from(broken.value()).unwrap();
    rusqlite::Connection::open(&paths.sqlite_database)
        .unwrap()
        .execute(
            "UPDATE agent_history_items SET item_json = '{broken' WHERE session_id = ?",
            [broken_id],
        )
        .unwrap();

    let (workers, reopened) = reopen(&paths);
    assert_eq!(
        refused(&reopened)
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![broken]
    );
    assert!(transcript_refused(&reopened, broken));
    serves(&reopened, intact, "intact");
    // Nothing is swept while a session's references are unknown.
    assert!(blob_path(&paths, &kept).is_file());
    close(reopened, workers, &[]);
    let stored: String = rusqlite::Connection::open(&paths.sqlite_database)
        .unwrap()
        .query_row(
            "SELECT item_json FROM agent_history_items WHERE session_id = ?",
            [broken_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, "{broken");
}

#[test]
fn an_ordinal_violation_still_rejects_its_session_on_every_start() {
    let (_temporary, paths, workers, store) = open_store();
    let broken = store.allocate_session_id().unwrap();
    let intact = store.allocate_session_id().unwrap();
    create_with_text(&store, broken, "broken");
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(broken, 1, 2),
            history_mutations: Vec::new(),
        })
        .unwrap();
    create_with_text(&store, intact, "intact");
    close(store, workers, &[broken, intact]);
    let path = rollout_of(&paths, broken);
    let skipped = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| {
            let mut record: serde_json::Value = serde_json::from_str(line).unwrap();
            if record["ordinal"] == 1 {
                record["ordinal"] = serde_json::json!(5);
            }
            format!("{}\n", serde_json::to_string(&record).unwrap())
        })
        .collect::<String>();
    fs::write(&path, &skipped).unwrap();
    fs::remove_file(&paths.sqlite_database).unwrap();

    // The rebuild rejects it, and the start after, on the kept index,
    // retries its recorded repair and rejects it again.
    for _ in 0..2 {
        let (workers, reopened) = reopen(&paths);
        let refused_sessions = refused(&reopened);
        assert_eq!(
            refused_sessions
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            vec![broken]
        );
        assert!(refused_sessions[0]
            .1
            .contains("expected record ordinal 1, got 5"));
        assert!(reopened.load_runtime_state(broken).is_err());
        serves(&reopened, intact, "intact");
        assert!(reopened.allocate_session_id().unwrap().value() > intact.value());
        close(reopened, workers, &[]);
        assert_eq!(fs::read_to_string(&path).unwrap(), skipped);
    }
}

#[test]
fn a_session_with_two_lifecycle_copies_is_quarantined_with_both_kept() {
    let (_temporary, paths, workers, store) = open_store();
    let doubled = store.allocate_session_id().unwrap();
    let intact = store.allocate_session_id().unwrap();
    create_with_text(&store, doubled, "doubled");
    let mut archived = runtime(doubled, 1, 2);
    archived.lifecycle = AgentSessionLifecycle::Archived;
    store
        .append_commit(AgentConversationCommit {
            runtime: archived,
            history_mutations: Vec::new(),
        })
        .unwrap();
    create_with_text(&store, intact, "intact");
    close(store, workers, &[doubled, intact]);
    let active = rollout_of(&paths, doubled);
    let copy = paths.archived_sessions.join(active.file_name().unwrap());
    fs::copy(&active, &copy).unwrap();

    let (workers, reopened) = reopen(&paths);
    let refused_sessions = refused(&reopened);
    assert_eq!(
        refused_sessions
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![doubled]
    );
    assert!(refused_sessions[0]
        .1
        .contains("both active and archived copies"));
    serves(&reopened, intact, "intact");
    assert!(active.is_file() && copy.is_file());
    close(reopened, workers, &[]);
}

pub(super) fn stage(
    store: &FileAgentHistoryStore,
    session_id: AgentSessionId,
    width: u32,
) -> AgentImageAttachmentRef {
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(width, 1)
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    store
        .stage_image_attachment(
            session_id,
            hanafubuki_agent_kernel::PreparedAgentImage {
                bytes: output.into_inner(),
                mime_type: "image/png".to_string(),
                width,
                height: 1,
                display_name: "image.png".to_string(),
                detail: hanafubuki_agent_kernel::AgentImageDetail::Auto,
            },
        )
        .unwrap()
}

/// Rewrites every record of `path` with `format_version` and returns what
/// the file held.
pub(super) fn set_format_version(path: &std::path::Path, format_version: u32) -> String {
    let original = fs::read_to_string(path).unwrap();
    let rewritten = original
        .lines()
        .map(|line| {
            let mut record: serde_json::Value = serde_json::from_str(line).unwrap();
            record["formatVersion"] = serde_json::json!(format_version);
            format!("{}\n", serde_json::to_string(&record).unwrap())
        })
        .collect::<String>();
    fs::write(path, rewritten).unwrap();
    original
}

/// A fork child's items can reference a blob its parent staged (a restored
/// prompt's images keep their source session). Once the child's rollout is
/// moved aside, the index no longer holds that reference, but the moved
/// file still does, so the sweep keeps the blob: the child reads it again
/// once restored. A blob no file references is still swept.
#[test]
fn a_parent_blob_a_quarantined_fork_child_references_is_never_swept() {
    let (_temporary, paths, workers, store) = open_store();
    let parent = store.allocate_session_id().unwrap();
    create_with_text(&store, parent, "parent");
    let shared = stage(&store, parent, 1);
    let child = store.allocate_session_id().unwrap();
    let mut child_runtime = runtime(child, 0, 3);
    child_runtime.forked_from_session_id = Some(parent);
    let mut resent = item(2, 0, "");
    resent.source_session_id = child;
    resent.content = ConversationItem::UserMessage {
        text: "again".to_string(),
        images: vec![shared.clone()],
    };
    store
        .create_fork_at_head(
            parent,
            AgentRevision::new(0),
            AgentConversationCommit {
                runtime: child_runtime,
                history_mutations: vec![AgentHistoryMutation::AppendItem { item: resent }],
            },
        )
        .unwrap();
    let later = store.allocate_session_id().unwrap();
    create_with_text(&store, later, "later");
    let unreferenced = stage(&store, later, 2);
    close(store, workers, &[parent, child, later]);

    let path = rollout_of(&paths, child);
    let original = set_format_version(
        &path,
        hanafubuki_agent_kernel::AGENT_HISTORY_FORMAT_VERSION + 1,
    );
    fs::remove_file(&paths.sqlite_database).unwrap();
    let (workers, rebuilt) = reopen(&paths);
    assert_eq!(refused(&rebuilt).len(), 1);
    close(rebuilt, workers, &[]);

    // Nothing is refused on the kept index, so the sweep runs.
    let (workers, reopened) = reopen(&paths);
    assert!(refused(&reopened).is_empty());
    assert!(blob_path(&paths, &shared).is_file());
    assert!(!blob_path(&paths, &unreferenced).exists());
    serves(&reopened, parent, "parent");
    close(reopened, workers, &[]);

    // Restored from quarantine, the child reads its parent's blob.
    let moved = quarantined_files(&paths.active_sessions);
    assert_eq!(moved.len(), 1);
    fs::remove_file(&moved[0]).unwrap();
    fs::write(&path, original).unwrap();
    fs::remove_file(&paths.sqlite_database).unwrap();
    let (workers, restored) = reopen(&paths);
    assert!(refused(&restored).is_empty());
    assert_eq!(
        restored
            .load_visible_image_attachment(child, &shared)
            .unwrap(),
        fs::read(blob_path(&paths, &shared)).unwrap()
    );
    close(restored, workers, &[]);
}

/// Every listed session id, page by page.
fn listed(store: &FileAgentHistoryStore, page_size: usize) -> Vec<AgentSessionId> {
    let mut listed = Vec::new();
    let mut cursor = None;
    loop {
        let page = store
            .list_sessions_page(AgentSessionListRequest {
                page_size,
                cursor,
                ..AgentSessionListRequest::default()
            })
            .unwrap();
        listed.extend(page.sessions.iter().map(|summary| summary.session_id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            return listed;
        }
    }
}

#[test]
fn a_summary_that_does_not_decode_costs_only_its_row_of_the_session_list() {
    let (_temporary, paths, workers, store) = open_store();
    let sessions = (0..3)
        .map(|index| {
            let session_id = store.allocate_session_id().unwrap();
            create_with_text(&store, session_id, &format!("session {index}"));
            session_id
        })
        .collect::<Vec<_>>();
    let broken = sessions[1];
    rusqlite::Connection::open(&paths.sqlite_database)
        .unwrap()
        .execute(
            "UPDATE agent_sessions SET summary_json = '{broken' WHERE session_id = ?",
            [i64::try_from(broken.value()).unwrap()],
        )
        .unwrap();

    // Paged one at a time, the cursor passes the left-out row.
    let expected = vec![sessions[2], sessions[0]];
    assert_eq!(listed(&store, 1), expected);
    assert_eq!(listed(&store, 10), expected);
    assert!(transcript_refused(&store, broken));
    let refused_sessions = refused(&store);
    assert_eq!(refused_sessions.len(), 1);
    assert_eq!(refused_sessions[0].0, broken);
    assert!(refused_sessions[0].1.contains("summary"));
    serves(&store, sessions[0], "session 0");
    close(store, workers, &sessions);
}

#[derive(Default)]
struct RecordingRefusals(parking_lot::Mutex<Vec<hanafubuki_agent_kernel::AgentQuarantinedSession>>);

impl crate::AgentHistoryRefusalSink for RecordingRefusals {
    fn session_refused(&self, session: hanafubuki_agent_kernel::AgentQuarantinedSession) {
        self.0.lock().push(session);
    }
}

#[test]
fn a_session_the_list_refuses_is_told_to_the_refusal_sink_once() {
    let temporary = tempfile::tempdir().unwrap();
    let paths = AgentHistoryPaths::under(temporary.path().join("agent"));
    let workers = BackgroundWorkerGroup::default();
    let refusals = Arc::new(RecordingRefusals::default());
    let store = Arc::new(
        FileAgentHistoryStore::open_reporting_refusals(
            paths.clone(),
            workers.clone(),
            refusals.clone(),
        )
        .unwrap(),
    );
    let broken = store.allocate_session_id().unwrap();
    let intact = store.allocate_session_id().unwrap();
    create_with_text(&store, broken, "broken");
    create_with_text(&store, intact, "intact");
    assert_eq!(listed(&store, 10), vec![intact, broken]);
    assert!(refusals.0.lock().is_empty());
    rusqlite::Connection::open(&paths.sqlite_database)
        .unwrap()
        .execute(
            "UPDATE agent_sessions SET summary_json = '{broken' WHERE session_id = ?",
            [i64::try_from(broken.value()).unwrap()],
        )
        .unwrap();

    assert_eq!(listed(&store, 10), vec![intact]);
    assert_eq!(listed(&store, 1), vec![intact]);
    let told = refusals.0.lock().clone();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].session_id, broken);
    assert!(told[0].reason.contains("summary"));
    assert_eq!(
        refused(&store),
        vec![(
            broken,
            format!("Agent history projection unavailable: {}", told[0].reason)
        )]
    );
    close(store, workers, &[broken, intact]);
}

/// Makes every projection of an append fail until the trigger is dropped.
fn fail_projections(paths: &AgentHistoryPaths, fail: bool) {
    let connection = rusqlite::Connection::open(&paths.sqlite_database).unwrap();
    let statement = if fail {
        "CREATE TRIGGER fail_projection BEFORE INSERT ON agent_history_records BEGIN SELECT RAISE(ABORT, 'injected projection failure'); END;"
    } else {
        "DROP TRIGGER fail_projection;"
    };
    connection.execute_batch(statement).unwrap();
}

#[test]
fn a_session_whose_append_lagged_is_served_again_once_its_writer_catches_up() {
    let (_temporary, paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    create_with_text(&store, session_id, "lagged");
    fail_projections(&paths, true);
    // The append is durable; only its projection failed.
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(session_id, 1, 2),
            history_mutations: Vec::new(),
        })
        .unwrap();
    assert!(transcript_refused(&store, session_id));
    assert!(store.load_runtime_state(session_id).is_err());
    fail_projections(&paths, false);

    // Stopping the writer projects the rest, without a restart.
    store.shutdown_session(session_id).unwrap();
    assert!(!transcript_refused(&store, session_id));
    assert_eq!(
        store.load_runtime_state(session_id).unwrap().revision,
        AgentRevision::new(1)
    );
    assert!(refused(&store).is_empty());
    close(store, workers, &[]);
}

#[test]
fn a_session_refused_at_open_stays_refused_when_its_writer_appends_and_stops() {
    let (_temporary, paths, workers, store) = open_store();
    let broken = store.allocate_session_id().unwrap();
    create_with_text(&store, broken, "broken");
    let missing = commit_image(&store, broken);
    close(store, workers, &[broken]);
    fs::remove_file(blob_path(&paths, &missing)).unwrap();

    let (workers, reopened) = reopen(&paths);
    assert_eq!(refused(&reopened).len(), 1);
    reopened
        .append_commit(AgentConversationCommit {
            runtime: runtime(broken, 2, 3),
            history_mutations: Vec::new(),
        })
        .unwrap();
    reopened.shutdown_session(broken).unwrap();
    let refused_sessions = refused(&reopened);
    assert_eq!(refused_sessions.len(), 1);
    assert!(refused_sessions[0].1.contains("could not be read"));
    assert!(transcript_refused(&reopened, broken));
    assert!(reopened.load_runtime_state(broken).is_err());
    close(reopened, workers, &[]);
}
