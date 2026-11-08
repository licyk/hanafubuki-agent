use std::fs;

use super::*;

#[test]
fn candidate_search_is_ignore_aware_and_bounded() {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir_all(temporary.path().join("visible")).unwrap();
    fs::create_dir_all(temporary.path().join("ignored")).unwrap();
    fs::write(temporary.path().join(".gitignore"), "ignored/\n").unwrap();
    fs::write(temporary.path().join("visible/model.yaml"), "ok").unwrap();
    fs::write(temporary.path().join("ignored/model-secret.yaml"), "hidden").unwrap();

    let result = search_roots(
        &[("fixture".to_string(), temporary.path().to_path_buf())],
        "model",
        1,
    )
    .unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].label, "visible/model.yaml");
    assert!(result.truncated);
}
