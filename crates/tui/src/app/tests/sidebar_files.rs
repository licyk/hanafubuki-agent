use super::*;

#[test]
fn newly_committed_file_links_join_the_open_chooser_without_duplicate_entries() {
    let mut h = Harness::start_timed(
        vec![
            (assistant_events("First [link](one.txt)."), Duration::ZERO),
            (
                assistant_events("Second [link](two.txt) and [again](one.txt)."),
                Duration::from_millis(500),
            ),
        ],
        true,
        TranscriptMode::Owned,
    );
    h.answered("first", "First");
    h.type_and_submit("second");
    h.type_and_submit("/sidebar");
    h.pump_until("history and newly committed references in Files", |h| {
        let files = h.app.workspace.sidebar.files.entries();
        ["one.txt", "two.txt"]
            .into_iter()
            .all(|name| files.iter().any(|f| f.label == name))
    });
    let files = h.app.workspace.sidebar.files.entries();
    assert_eq!(files.iter().filter(|f| f.label == "one.txt").count(), 1);
}
