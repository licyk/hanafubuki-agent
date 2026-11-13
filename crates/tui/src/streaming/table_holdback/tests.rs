// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

#[test]
fn table_holdback_state_detects_header_plus_delimiter() {
    let source = "| Key | Description |\n| --- | --- |\n";
    assert!(matches!(
        table_holdback_state(source),
        TableHoldbackState::Confirmed { .. }
    ));
}

#[test]
fn table_holdback_state_detects_single_column_header_plus_delimiter() {
    let source = "| Only |\n| --- |\n";
    assert!(matches!(
        table_holdback_state(source),
        TableHoldbackState::Confirmed { .. }
    ));
}

#[test]
fn table_holdback_state_ignores_table_like_lines_inside_unclosed_long_fence() {
    let source = "````sh\n```cmd\n| Key | Description |\n| --- | --- |\n````\n";
    assert!(
        matches!(table_holdback_state(source), TableHoldbackState::None),
        "table holdback should ignore pipe lines inside an open non-markdown fence",
    );
}

#[test]
fn table_holdback_state_treats_indented_fence_text_as_plain_content() {
    let source = "    ```sh\n| Key | Description |\n| --- | --- |\n";
    assert!(
        matches!(
            table_holdback_state(source),
            TableHoldbackState::Confirmed { .. }
        ),
        "indented fence-like text should not open a fence and should not block table detection",
    );
}

#[test]
fn table_holdback_state_ignores_table_like_lines_inside_blockquoted_other_fence() {
    let source = "> ```sh\n> | Key | Value |\n> | --- | --- |\n> ```\n";
    assert!(
        matches!(table_holdback_state(source), TableHoldbackState::None),
        "table holdback should ignore pipe lines inside non-markdown blockquoted fences",
    );
}

#[test]
fn incremental_holdback_matches_stateless_scan_per_chunk() {
    let chunks = [
        "status | owner\n",
        "\n",
        "> ```sh\n",
        "> | A | B |\n",
        "> | --- | --- |\n",
        "> ```\n",
        "> | Key | Value |\n",
        "> | --- | --- |\n",
    ];

    let mut scanner = TableHoldbackScanner::new();
    let mut source = String::new();
    for chunk in chunks {
        source.push_str(chunk);
        scanner.scan(&source, 0);
        assert_eq!(
            scanner.state(),
            table_holdback_state(&source),
            "scanner mismatch after chunk: {chunk:?}\nsource:\n{source}",
        );
    }
}

#[test]
fn incremental_holdback_detects_header_delimiter_across_chunk_boundary() {
    let mut scanner = TableHoldbackScanner::new();
    scanner.scan("| A | B |\n", 0);
    assert_eq!(
        scanner.state(),
        TableHoldbackState::PendingHeader { header_start: 0 }
    );
    scanner.scan("| A | B |\n| --- | --- |\n", 0);
    assert_eq!(
        scanner.state(),
        TableHoldbackState::Confirmed { table_start: 0 }
    );
}
