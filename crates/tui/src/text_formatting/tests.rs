// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;

#[test]
fn test_center_truncate_doesnt_truncate_short_path() {
    let sep = std::path::MAIN_SEPARATOR;
    let path = format!("{sep}Users{sep}codex{sep}Public");
    let truncated = center_truncate_path(&path, 40);

    assert_eq!(truncated, path);
}

#[test]
fn test_center_truncate_truncates_long_path() {
    let sep = std::path::MAIN_SEPARATOR;
    let path = format!("~{sep}hello{sep}the{sep}fox{sep}is{sep}very{sep}fast");
    let truncated = center_truncate_path(&path, 24);

    assert_eq!(
        truncated,
        format!("~{sep}hello{sep}the{sep}…{sep}very{sep}fast")
    );
}

#[test]
fn test_center_truncate_truncates_long_windows_path() {
    let sep = std::path::MAIN_SEPARATOR;
    let path = format!(
        "C:{sep}Users{sep}codex{sep}Projects{sep}super{sep}long{sep}windows{sep}path{sep}file.txt"
    );
    let truncated = center_truncate_path(&path, 36);

    let expected = format!("C:{sep}Users{sep}codex{sep}…{sep}path{sep}file.txt");

    assert_eq!(truncated, expected);
}

#[test]
fn test_center_truncate_handles_long_segment() {
    let sep = std::path::MAIN_SEPARATOR;
    let path = format!("~{sep}supercalifragilisticexpialidocious");
    let truncated = center_truncate_path(&path, 18);

    assert_eq!(truncated, format!("~{sep}…cexpialidocious"));
}

#[test]
fn strip_terminal_controls_drops_escapes_and_control_characters() {
    assert_eq!(
        strip_terminal_controls("\u{1b}[31mred\u{1b}[0m\r\nnext\tline\u{7}"),
        "red\nnext\tline"
    );
    assert_eq!(
        strip_terminal_controls("\u{1b}]0;title\u{7}text \u{1b}]8;;url\u{1b}\\link"),
        "text link"
    );
}
