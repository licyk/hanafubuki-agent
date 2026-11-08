//! A session file the history keeps but the index does not hold still
//! needs the session it forks from and the attachments it names, so that
//! session is not deleted: the deletion is refused, naming the file, and
//! nothing of either is touched.

use super::quarantine::{
    blob_path, close, create_with_text, quarantined_files, refused, reopen, rollout_of, serves,
    set_format_version, stage,
};
use super::*;
use hanafubuki_agent_kernel::{AgentHistoryStoreError, AgentImageAttachmentRef};

/// A parent, an image it staged and a fork child at its head whose item
/// resends that image.
fn parent_with_fork(
    store: &FileAgentHistoryStore,
) -> (AgentSessionId, AgentSessionId, AgentImageAttachmentRef) {
    let parent = store.allocate_session_id().unwrap();
    create_with_text(store, parent, "parent");
    let shared = stage(store, parent, 1);
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
    (parent, child, shared)
}

/// The dependents a refused deletion of `session_id` names.
fn dependents_refusing(store: &FileAgentHistoryStore, session_id: AgentSessionId) -> Vec<String> {
    let checked = store.check_sessions_deletable(&[session_id]);
    let deleted = store.delete_sessions_confirmed(&[session_id]);
    assert_eq!(checked, deleted);
    match deleted {
        Err(AgentHistoryStoreError::ReferencedByQuarantinedSession {
            session_id: refused,
            dependents,
        }) if refused == session_id => dependents,
        other => panic!("the deletion of session {session_id} was not refused: {other:?}"),
    }
}

#[test]
fn a_parent_a_quarantined_fork_child_needs_is_not_deleted() {
    let (_temporary, paths, workers, store) = open_store();
    let (parent, child, shared) = parent_with_fork(&store);
    let alone = store.allocate_session_id().unwrap();
    create_with_text(&store, alone, "alone");
    close(store, workers, &[parent, child, alone]);
    let child_rollout = rollout_of(&paths, child);
    let parent_rollout = rollout_of(&paths, parent);
    set_format_version(
        &child_rollout,
        hanafubuki_agent_kernel::AGENT_HISTORY_FORMAT_VERSION + 1,
    );
    fs::remove_file(&paths.sqlite_database).unwrap();
    let (workers, rebuilt) = reopen(&paths);
    close(rebuilt, workers, &[]);

    // On the kept index nothing is refused, and no lineage row names the
    // child: only the moved file says it forks from the parent.
    let (workers, reopened) = reopen(&paths);
    assert!(refused(&reopened).is_empty());
    let moved = quarantined_files(&paths.active_sessions);
    assert_eq!(moved.len(), 1);
    let dependents = dependents_refusing(&reopened, parent);
    assert_eq!(
        dependents,
        vec![format!("session {child} at {}", moved[0].display())]
    );
    let message = reopened
        .delete_sessions_confirmed(&[parent])
        .unwrap_err()
        .to_string();
    assert!(message.contains(&format!("Agent session {parent} cannot be deleted")));
    assert!(message.contains("restore or remove them first"));
    assert!(fs::read_dir(&paths.trash).unwrap().next().is_none());
    assert!(parent_rollout.is_file());
    assert!(moved[0].is_file());
    assert!(blob_path(&paths, &shared).is_file());
    serves(&reopened, parent, "parent");

    // A session nothing forks from deletes as before.
    reopened.check_sessions_deletable(&[alone]).unwrap();
    reopened.delete_sessions_confirmed(&[alone]).unwrap();
    assert!(matches!(
        reopened.load_runtime_state(alone),
        Err(AgentHistoryStoreError::NotFound { .. })
    ));

    // Once the moved file is gone, nothing keeps the parent.
    fs::remove_file(&moved[0]).unwrap();
    reopened.delete_sessions_confirmed(&[parent]).unwrap();
    assert!(!parent_rollout.exists());
    assert!(!blob_path(&paths, &shared).exists());
    close(reopened, workers, &[]);
}

/// A fork child whose rollout does not project stays in place with a
/// pending repair; rebuilt, the index holds no lineage row for it.
#[test]
fn a_parent_a_fork_child_refused_at_open_needs_is_not_deleted() {
    let (_temporary, paths, workers, store) = open_store();
    let (parent, child, shared) = parent_with_fork(&store);
    store
        .append_commit(AgentConversationCommit {
            runtime: {
                let mut later = runtime(child, 1, 3);
                later.forked_from_session_id = Some(parent);
                later
            },
            history_mutations: Vec::new(),
        })
        .unwrap();
    close(store, workers, &[parent, child]);
    let child_rollout = rollout_of(&paths, child);
    let skipped = fs::read_to_string(&child_rollout)
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
    fs::write(&child_rollout, &skipped).unwrap();
    fs::remove_file(&paths.sqlite_database).unwrap();

    let (workers, reopened) = reopen(&paths);
    assert_eq!(
        refused(&reopened)
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![child]
    );
    let dependents = dependents_refusing(&reopened, parent);
    assert_eq!(
        dependents,
        vec![format!("session {child} at {}", child_rollout.display())]
    );
    assert_eq!(fs::read_to_string(&child_rollout).unwrap(), skipped);
    assert!(rollout_of(&paths, parent).is_file());
    assert!(blob_path(&paths, &shared).is_file());
    serves(&reopened, parent, "parent");
    close(reopened, workers, &[]);
}

/// A kept file that names one of a session's blobs keeps the session (its
/// attachment directory goes with it), whichever session the file is.
#[test]
fn a_session_whose_blob_a_quarantined_file_names_is_not_deleted() {
    let (_temporary, paths, workers, store) = open_store();
    let owner = store.allocate_session_id().unwrap();
    create_with_text(&store, owner, "owner");
    let named = stage(&store, owner, 1);
    let other = store.allocate_session_id().unwrap();
    create_with_text(&store, other, "other");
    let recovery = paths.active_sessions.join(".recovery");
    fs::create_dir_all(&recovery).unwrap();
    let foreign = recovery.join("foreign.jsonl.1.0.invalid");
    fs::write(
        &foreign,
        format!(
            "{{\"image\": {{\"digest\": \"{}\"}}}}\n",
            named.attachment_id.as_str()
        ),
    )
    .unwrap();

    assert_eq!(
        dependents_refusing(&store, owner),
        vec![foreign.display().to_string()]
    );
    assert!(blob_path(&paths, &named).is_file());
    store.delete_sessions_confirmed(&[other]).unwrap();
    close(store, workers, &[]);
}
