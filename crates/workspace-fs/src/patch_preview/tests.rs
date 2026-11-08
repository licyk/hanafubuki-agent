use super::*;

#[test]
fn an_update_keeps_the_order_the_model_wrote() {
    let preview = preview_patch(
        "*** Begin Patch\n\
         *** Update File: src/app.py\n\
         @@ def main():\n\
         \x20    setup()\n\
         -    old_call()\n\
         +    new_call()\n\
         +    extra_call()\n\
         \x20    teardown()\n\
         *** End Patch",
    );

    assert_eq!(preview.error, None);
    let [file] = preview.files.as_slice() else {
        panic!("expected one file, got {:?}", preview.files);
    };
    assert_eq!(file.operation, PatchPreviewOperation::Update);
    assert_eq!(file.path, "src/app.py");
    assert_eq!(file.added, 2);
    assert_eq!(file.removed, 1);

    // Splitting into old/new lines is lossy; the preview must show the original
    // interleaving, not a reconstruction of it.
    let rendered = file
        .lines
        .iter()
        .map(|line| (line.kind, line.text.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        rendered,
        vec![
            (PatchPreviewLineKind::Hunk, "def main():"),
            (PatchPreviewLineKind::Context, "    setup()"),
            (PatchPreviewLineKind::Removed, "    old_call()"),
            (PatchPreviewLineKind::Added, "    new_call()"),
            (PatchPreviewLineKind::Added, "    extra_call()"),
            (PatchPreviewLineKind::Context, "    teardown()"),
        ]
    );
}

#[test]
fn an_add_renders_every_line_as_an_addition() {
    let preview =
        preview_patch("*** Begin Patch\n*** Add File: notes.txt\n+first\n+second\n*** End Patch");

    let [file] = preview.files.as_slice() else {
        panic!("expected one file");
    };
    assert_eq!(file.operation, PatchPreviewOperation::Add);
    assert_eq!(file.added, 2);
    assert_eq!(file.removed, 0);
    assert!(file
        .lines
        .iter()
        .all(|line| line.kind == PatchPreviewLineKind::Added));
}

#[test]
fn an_empty_add_renders_no_lines() {
    let preview = preview_patch("*** Begin Patch\n*** Add File: __init__.py\n*** End Patch");
    let [file] = preview.files.as_slice() else {
        panic!("expected one file");
    };
    assert!(file.lines.is_empty());
    assert_eq!(file.added, 0);
}

#[test]
fn a_delete_names_the_file_and_shows_no_lines() {
    let preview = preview_patch("*** Begin Patch\n*** Delete File: stale.txt\n*** End Patch");
    let [file] = preview.files.as_slice() else {
        panic!("expected one file");
    };
    assert_eq!(file.operation, PatchPreviewOperation::Delete);
    assert_eq!(file.path, "stale.txt");
    assert!(file.lines.is_empty());
}

#[test]
fn a_rename_carries_its_destination() {
    let preview = preview_patch(
        "*** Begin Patch\n\
         *** Update File: old.py\n\
         *** Move to: new.py\n\
         @@\n-a\n+b\n\
         *** End Patch",
    );
    let [file] = preview.files.as_slice() else {
        panic!("expected one file");
    };
    assert_eq!(file.moved_to.as_deref(), Some("new.py"));
}

#[test]
fn end_of_file_is_shown_as_a_marker() {
    let preview = preview_patch(
        "*** Begin Patch\n\
         *** Update File: app.py\n\
         @@\n-last\n+final\n*** End of File\n\
         *** End Patch",
    );
    let [file] = preview.files.as_slice() else {
        panic!("expected one file");
    };
    assert!(file
        .lines
        .iter()
        .any(|line| line.kind == PatchPreviewLineKind::Marker));
}

/// The whole point of moving the reading into Rust: a patch the tool rejects
/// must not render as a clean diff. The interface used to show `+1 −1` on a
/// file the backend had refused, with nothing on screen to explain it.
#[test]
fn a_patch_the_parser_rejects_yields_no_files_and_an_explanation() {
    for malformed in [
        "not a patch at all",
        "*** Begin Patch\n*** Add File: a.txt\nmissing the plus\n*** End Patch",
        "*** Begin Patch\n*** Update File: a.txt\n*** End Patch",
        "*** Begin Patch\n*** End Patch",
    ] {
        let preview = preview_patch(malformed);
        assert!(
            preview.files.is_empty(),
            "{malformed:?} rendered files despite being rejected"
        );
        assert!(
            preview.error.is_some(),
            "{malformed:?} was rejected without an explanation"
        );
    }
}

/// A truncated preview must still say so rather than silently showing less.
#[test]
fn a_very_long_file_reports_the_lines_it_omitted() {
    let body = (0..600)
        .map(|index| format!("+line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let preview = preview_patch(&format!(
        "*** Begin Patch\n*** Add File: big.txt\n{body}\n*** End Patch"
    ));
    let [file] = preview.files.as_slice() else {
        panic!("expected one file");
    };
    assert_eq!(file.lines.len(), 500);
    assert_eq!(file.omitted_lines, 100);
    // The counts describe the whole patch, not just the painted part.
    assert_eq!(file.added, 600);
}

/// Multiple files keep their order, so the preview reads like the patch.
#[test]
fn files_keep_patch_order() {
    let preview = preview_patch(
        "*** Begin Patch\n\
         *** Add File: one.txt\n+a\n\
         *** Delete File: two.txt\n\
         *** Add File: three.txt\n+c\n\
         *** End Patch",
    );
    let paths = preview
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["one.txt", "two.txt", "three.txt"]);
}
