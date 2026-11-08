//! One note record that does not read costs that record, never the log.

use super::*;
use crate::AgentMemoryContributionMode;

fn request(content: &str) -> AppendAdHocMemoryNote {
    AppendAdHocMemoryNote {
        created_at_millis: 150,
        session_id: "baseline".into(),
        turn_id: 7,
        source_revision: 11,
        operation: AdHocMemoryNoteOperation::Remember,
        content: content.into(),
    }
}

fn opened() -> (tempfile::TempDir, AgentMemoryRepository) {
    let temporary = tempfile::TempDir::new().unwrap();
    let repository =
        AgentMemoryRepository::open(AgentMemoryPaths::under(temporary.path().join("memory")))
            .unwrap();
    repository
        .sync_source_state("baseline", AgentMemoryContributionMode::Enabled, 1, 11)
        .unwrap();
    (temporary, repository)
}

fn append_raw(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
}

#[test]
fn a_bad_note_line_is_skipped_and_the_other_notes_read() {
    let (_temporary, repository) = opened();
    let path = repository.paths().ad_hoc_notes.clone();
    let first = repository
        .append_ad_hoc_note(request("first note"))
        .unwrap();
    let first_line = fs::read(&path).unwrap();

    // A torn append, bytes that are not UTF-8, a record whose content was
    // changed after its ID was derived, and a repeat of the first note.
    append_raw(&path, b"{\"version\":1,\"noteId\":\"ab\n");
    append_raw(&path, b"\xff\xfe not text\n");
    let mut tampered = first.clone();
    tampered.content = "changed after the fact".into();
    let mut tampered = serde_json::to_vec(&tampered).unwrap();
    tampered.push(b'\n');
    append_raw(&path, &tampered);
    append_raw(&path, &first_line);

    let second = repository
        .append_ad_hoc_note(request("second note"))
        .unwrap();
    let notes = repository.active_ad_hoc_notes().unwrap();
    assert_eq!(notes, vec![first.clone(), second.clone()]);

    // Opening again recovers over the same log.
    let reopened = AgentMemoryRepository::open(repository.paths().clone()).unwrap();
    assert_eq!(reopened.active_ad_hoc_notes().unwrap(), vec![first, second]);
}

#[test]
fn a_log_past_its_byte_bound_is_still_corrupt() {
    let (_temporary, repository) = opened();
    let path = repository.paths().ad_hoc_notes.clone();
    let file = fs::File::create(&path).unwrap();
    file.set_len(MAX_AD_HOC_MEMORY_LOG_BYTES + 1).unwrap();

    assert!(matches!(
        repository.active_ad_hoc_notes(),
        Err(AgentMemoryError::Corrupt(_))
    ));
}
