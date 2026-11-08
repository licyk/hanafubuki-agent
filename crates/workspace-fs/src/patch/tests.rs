use super::*;

#[test]
fn parses_codex_file_operations_and_move() {
    let patch = "*** Begin Patch\n*** Add File: new.txt\n+new\n*** Update File: old.txt\n*** Move to: moved.txt\n@@ heading\n-old\n+updated\n*** Delete File: gone.txt\n*** End Patch";
    let hunks = parse_patch(patch).unwrap().hunks;
    assert_eq!(hunks.len(), 3);
    assert!(
        matches!(&hunks[0], PatchHunk::Add { path, content } if path == &PathBuf::from("new.txt") && content == "new\n")
    );
    assert!(
        matches!(&hunks[1], PatchHunk::Update { path, move_to: Some(destination), .. } if path == &PathBuf::from("old.txt") && destination == &PathBuf::from("moved.txt"))
    );
    assert!(matches!(&hunks[2], PatchHunk::Delete { path } if path == &PathBuf::from("gone.txt")));
}

#[test]
fn parser_reports_model_actionable_line_errors() {
    let error =
        parse_patch("*** Begin Patch\n*** Add File: broken.txt\nmissing-prefix\n*** End Patch")
            .unwrap_err();
    assert!(error.to_string().contains("line 3"));
    assert!(error.to_string().contains("must start with '+'"));
}

#[test]
fn update_matching_is_ordered_and_whitespace_tolerant() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@ section\n-value\n+changed\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    let output =
        derive_updated_content("section  \n  value  \ntail\n", "sample.txt", chunks).unwrap();
    assert_eq!(output, "section  \nchanged\ntail\n");
}

#[test]
fn update_rejects_missing_context_without_mutation_plan() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@ absent\n-old\n+new\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    assert!(matches!(
        derive_updated_content("present\nold\n", "sample.txt", chunks),
        Err(WorkspaceFsError::PreimageMismatch(_))
    ));
}

#[test]
fn later_chunks_cannot_match_lines_before_an_earlier_chunk() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-second\n+changed\n@@\n-first\n+changed\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    assert!(matches!(
        derive_updated_content("first\nsecond\n", "sample.txt", chunks),
        Err(WorkspaceFsError::PreimageMismatch(_))
    ));
}

#[test]
fn update_context_may_contain_patch_marker_text() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n *** End Patch\n-old\n+new\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    assert_eq!(
        derive_updated_content("*** End Patch\nold\n", "sample.txt", chunks).unwrap(),
        "*** End Patch\nnew\n"
    );
}

#[test]
fn update_matching_normalizes_common_model_punctuation() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-Use \"apply-patch\"\n+Use \"apply_patch\"\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    assert_eq!(
        derive_updated_content("Use “apply‐patch”\n", "sample.txt", chunks).unwrap(),
        "Use \"apply_patch\"\n"
    );
}

/// `apply_patch` is the only file-writing tool, so an empty file has to be
/// expressible. An `Add File` hunk with no `+` lines is that expression — `+`
/// alone is a blank line, which is a one-byte file, not an empty one.
#[test]
fn an_add_hunk_without_content_lines_creates_an_empty_file() {
    let hunks = parse_patch("*** Begin Patch\n*** Add File: pkg/__init__.py\n*** End Patch")
        .unwrap()
        .hunks;

    assert!(
        matches!(&hunks[0], PatchHunk::Add { path, content } if path == &PathBuf::from("pkg/__init__.py") && content.is_empty())
    );

    // The neighbouring shapes still mean what they meant.
    let blank = parse_patch("*** Begin Patch\n*** Add File: a.txt\n+\n*** End Patch")
        .unwrap()
        .hunks;
    assert!(matches!(&blank[0], PatchHunk::Add { content, .. } if content == "\n"));

    // An empty add still composes with the hunks around it.
    let mixed = parse_patch("*** Begin Patch\n*** Add File: a/__init__.py\n*** Add File: a/main.py\n+run()\n*** End Patch",
    )
    .unwrap()
    .hunks;
    assert_eq!(mixed.len(), 2);
    assert!(matches!(&mixed[0], PatchHunk::Add { content, .. } if content.is_empty()));
    assert!(matches!(&mixed[1], PatchHunk::Add { content, .. } if content == "run()\n"));
}

/// Codex scenario 016: a chunk whose old lines are empty appends at end of
/// file, it does not insert at the current cursor. The local matcher used to
/// place it at the cursor (the file start for a leading chunk), which silently
/// wrote the added lines in the wrong place.
#[test]
fn pure_addition_chunk_appends_at_end_of_file() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n+added line 1\n+added line 2\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    assert_eq!(
        derive_updated_content("line1\nline2\n", "sample.txt", chunks).unwrap(),
        "line1\nline2\nadded line 1\nadded line 2\n"
    );
}

/// The same rule for the common `*** End of File` append pattern: the addition
/// must land at the end of the file, not the beginning.
#[test]
fn pure_addition_with_end_of_file_appends_at_file_end() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n+quux\n*** End of File\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    assert_eq!(
        derive_updated_content("foo\nbar\nbaz\n", "sample.txt", chunks).unwrap(),
        "foo\nbar\nbaz\nquux\n"
    );
}

/// A blank line after `*** End of File` is ignored, mirroring the Codex
/// streaming parser. The local parser used to record it as an empty context
/// line, which turned the following pure addition into a misplaced match.
#[test]
fn blank_lines_after_end_of_file_are_ignored() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n+appended\n*** End of File\n\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].is_end_of_file());
    assert_eq!(
        derive_updated_content("foo\nbar\n", "sample.txt", chunks).unwrap(),
        "foo\nbar\nappended\n"
    );
}

/// After `*** End of File` Codex still allows a new `@@` context chunk. The
/// local parser used to reject any non-empty line following the marker.
#[test]
fn a_context_chunk_may_follow_an_end_of_file_marker() {
    let hunks = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n+appended\n*** End of File\n@@\n-other\n+changed\n*** End Patch",
    )
    .unwrap()
    .hunks;
    let PatchHunk::Update { chunks, .. } = &hunks[0] else {
        panic!("expected update hunk");
    };
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].is_end_of_file());
    assert_eq!(
        derive_updated_content("first\nother\n", "sample.txt", chunks).unwrap(),
        "first\nchanged\nappended\n"
    );
}

/// A second `*** End of File` marker is rejected, matching the upstream
/// streaming parser, which only lets a blank line or a new `@@` header follow
/// an existing End of File chunk.
#[test]
fn a_second_end_of_file_marker_is_rejected() {
    let error = parse_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n+appended\n*** End of File\n*** End of File\n*** End Patch",
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("End of File must finish an update chunk"));
}
