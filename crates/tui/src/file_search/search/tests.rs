// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;
use nucleo::pattern::{AtomKind, Pattern};
use nucleo::Utf32Str;
use std::fs;
use std::sync::{Condvar, Mutex};
use tempfile::TempDir;

#[test]
fn verify_score_is_none_for_non_match() {
    let mut utf32buf = Vec::<char>::new();
    let line = "hello";
    let mut matcher = Matcher::new(Config::DEFAULT);
    let haystack: Utf32Str<'_> = Utf32Str::new(line, &mut utf32buf);
    let pattern = Pattern::new(
        "zzz",
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let score = pattern.score(haystack, &mut matcher);
    assert_eq!(score, None);
}

#[derive(Default)]
struct RecordingReporter {
    updates: Mutex<Vec<FileSearchSnapshot>>,
    update_times: Mutex<Vec<Instant>>,
    complete_times: Mutex<Vec<Instant>>,
    complete_cv: Condvar,
    update_cv: Condvar,
}

impl RecordingReporter {
    fn wait_for_complete(&self, timeout: Duration) -> bool {
        let completes = self.complete_times.lock().unwrap();
        if !completes.is_empty() {
            return true;
        }
        let (completes, _) = self.complete_cv.wait_timeout(completes, timeout).unwrap();
        !completes.is_empty()
    }
    fn clear(&self) {
        self.updates.lock().unwrap().clear();
        self.complete_times.lock().unwrap().clear();
    }

    fn updates(&self) -> Vec<FileSearchSnapshot> {
        self.updates.lock().unwrap().clone()
    }

    fn wait_for_updates_at_least(&self, min_len: usize, timeout: Duration) -> bool {
        let updates = self.updates.lock().unwrap();
        if updates.len() >= min_len {
            return true;
        }
        let (updates, _) = self.update_cv.wait_timeout(updates, timeout).unwrap();
        updates.len() >= min_len
    }

    fn snapshot(&self) -> FileSearchSnapshot {
        self.updates
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

impl SessionReporter for RecordingReporter {
    fn on_update(&self, snapshot: &FileSearchSnapshot) {
        let mut updates = self.updates.lock().unwrap();
        self.update_times.lock().unwrap().push(Instant::now());
        updates.push(snapshot.clone());
        self.update_cv.notify_all();
    }

    fn on_complete(&self) {
        {
            let mut complete_times = self.complete_times.lock().unwrap();
            complete_times.push(Instant::now());
        }
        self.complete_cv.notify_all();
    }
}

fn create_temp_tree(file_count: usize) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..file_count {
        let path = dir.path().join(format!("file-{i:04}.txt"));
        fs::write(path, format!("contents {i}")).unwrap();
    }
    dir
}

fn session(dir: &Path, reporter: &Arc<RecordingReporter>) -> FileSearchSession {
    create_session(vec![dir.to_path_buf()], reporter.clone())
}

#[test]
fn session_scanned_file_count_is_monotonic_across_queries() {
    let dir = create_temp_tree(200);
    let reporter = Arc::new(RecordingReporter::default());
    let session = session(dir.path(), &reporter);

    session.update_query("file-00");
    thread::sleep(Duration::from_millis(20));
    let first_snapshot = reporter.snapshot();
    session.update_query("file-01");
    thread::sleep(Duration::from_millis(20));
    let second_snapshot = reporter.snapshot();
    let _ = reporter.wait_for_complete(Duration::from_secs(5));
    let completed_snapshot = reporter.snapshot();

    assert!(second_snapshot.scanned_file_count >= first_snapshot.scanned_file_count);
    assert!(completed_snapshot.scanned_file_count >= second_snapshot.scanned_file_count);
}

#[test]
fn session_streams_updates_before_walk_complete() {
    let dir = create_temp_tree(600);
    let reporter = Arc::new(RecordingReporter::default());
    let session = session(dir.path(), &reporter);

    session.update_query("file-0");
    let completed = reporter.wait_for_complete(Duration::from_secs(5));

    assert!(completed);
    let updates = reporter.updates();
    assert!(updates.iter().any(|snapshot| !snapshot.walk_complete));
}

#[test]
fn session_accepts_query_updates_after_walk_complete() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "alpha").unwrap();
    fs::write(dir.path().join("beta.txt"), "beta").unwrap();
    let reporter = Arc::new(RecordingReporter::default());
    let session = session(dir.path(), &reporter);

    session.update_query("alpha");
    assert!(reporter.wait_for_complete(Duration::from_secs(5)));
    let updates_before = reporter.updates().len();

    session.update_query("beta");
    assert!(reporter.wait_for_updates_at_least(updates_before + 1, Duration::from_secs(5),));

    let updates = reporter.updates();
    let last_update = updates.last().cloned().expect("update");
    assert!(last_update
        .matches
        .iter()
        .any(|file_match| file_match.path.to_string_lossy().contains("beta.txt")));
}

#[test]
fn session_emits_complete_when_query_changes_with_no_matches() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "alpha").unwrap();
    fs::write(dir.path().join("beta.txt"), "beta").unwrap();
    let reporter = Arc::new(RecordingReporter::default());
    let session = session(dir.path(), &reporter);

    session.update_query("asdf");
    assert!(reporter.wait_for_complete(Duration::from_secs(5)));

    let completed_snapshot = reporter.snapshot();
    assert_eq!(completed_snapshot.matches, Vec::new());
    assert_eq!(completed_snapshot.total_match_count, 0);

    reporter.clear();

    session.update_query("asdfa");
    assert!(reporter.wait_for_complete(Duration::from_secs(5)));
    assert!(!reporter.updates().is_empty());
}

#[test]
fn session_emits_updates_when_query_changes() {
    let dir = create_temp_tree(200);
    let reporter = Arc::new(RecordingReporter::default());
    let session = session(dir.path(), &reporter);

    session.update_query("zzzzzzzz");
    let completed = reporter.wait_for_complete(Duration::from_secs(5));
    assert!(completed);

    reporter.clear();

    session.update_query("zzzzzzzzq");
    let completed = reporter.wait_for_complete(Duration::from_secs(5));
    assert!(completed);

    let updates = reporter.updates();
    assert_eq!(updates.len(), 1);
}

/// The matches of `query` once the session went idle after reporting them:
/// the walk may finish before or after the query reaches the matcher, and
/// either way the completion follows the query's last update.
fn completed_matches(roots: Vec<PathBuf>, query: &str) -> Vec<FileMatch> {
    let reporter = Arc::new(RecordingReporter::default());
    let session = create_session(roots, reporter.clone());
    session.update_query(query);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        {
            let updates = reporter.updates.lock().unwrap();
            let completes = reporter.complete_times.lock().unwrap();
            if let (Some((at, snapshot)), Some(completed)) = (
                reporter
                    .update_times
                    .lock()
                    .unwrap()
                    .last()
                    .zip(updates.last()),
                completes.last(),
            ) {
                if snapshot.query == query && completed >= at {
                    return snapshot.matches.clone();
                }
            }
        }
        assert!(Instant::now() < deadline, "the search never completed");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_session_finds_a_file_and_marks_what_matched() {
    let dir = create_temp_tree(40);
    let matches = completed_matches(vec![dir.path().to_path_buf()], "file-0000");

    let found = matches
        .iter()
        .find(|m| m.path == Path::new("file-0000.txt"))
        .expect("file-0000.txt is found");
    assert!(found.primary);
    assert_eq!(found.mention(), "file-0000.txt");
    assert!(!found.indices.is_empty());
    assert!(found.indices.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn ignored_files_are_not_offered() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
    fs::create_dir_all(dir.path().join("build")).unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("build/output.rs"), "").unwrap();
    fs::write(dir.path().join("src/output.rs"), "").unwrap();

    let matches = completed_matches(vec![dir.path().to_path_buf()], "output");

    let paths: Vec<_> = matches.iter().map(|m| m.path.clone()).collect();
    assert_eq!(paths, vec![Path::new("src").join("output.rs")]);
}

#[test]
fn a_nested_root_is_walked_once_and_another_roots_file_is_mentioned_whole() {
    // The standalone roots: the working directory first, then the Git root
    // that holds it.
    let outer = tempfile::tempdir().unwrap();
    let inner = outer.path().join("project");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("needle.rs"), "").unwrap();
    fs::write(outer.path().join("needle.md"), "").unwrap();

    let mut matches = completed_matches(vec![inner.clone(), outer.path().to_path_buf()], "needle");
    matches.sort_by(|a, b| a.path.cmp(&b.path));

    let mentions: Vec<_> = matches.iter().map(FileMatch::mention).collect();
    assert_eq!(
        mentions,
        vec![
            outer
                .path()
                .join("needle.md")
                .to_string_lossy()
                .into_owned(),
            "needle.rs".to_string(),
        ]
    );
}
