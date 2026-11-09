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

#[test]
fn test_truncate_text() {
    let text = "Hello, world!";
    let truncated = truncate_text(text, 8);
    assert_eq!(truncated, "Hello...");
}

#[test]
fn test_truncate_empty_string() {
    let text = "";
    let truncated = truncate_text(text, 5);
    assert_eq!(truncated, "");
}

#[test]
fn test_truncate_max_graphemes_zero() {
    let text = "Hello";
    let truncated = truncate_text(text, 0);
    assert_eq!(truncated, "");
}

#[test]
fn test_truncate_max_graphemes_one() {
    let text = "Hello";
    let truncated = truncate_text(text, 1);
    assert_eq!(truncated, "H");
}

#[test]
fn test_truncate_max_graphemes_two() {
    let text = "Hello";
    let truncated = truncate_text(text, 2);
    assert_eq!(truncated, "He");
}

#[test]
fn test_truncate_max_graphemes_three_boundary() {
    let text = "Hello";
    let truncated = truncate_text(text, 3);
    assert_eq!(truncated, "...");
}

#[test]
fn test_truncate_text_shorter_than_limit() {
    let text = "Hi";
    let truncated = truncate_text(text, 10);
    assert_eq!(truncated, "Hi");
}

#[test]
fn test_truncate_text_exact_length() {
    let text = "Hello";
    let truncated = truncate_text(text, 5);
    assert_eq!(truncated, "Hello");
}

#[test]
fn test_truncate_emoji() {
    let text = "👋🌍🚀✨💫";
    let truncated = truncate_text(text, 3);
    assert_eq!(truncated, "...");

    let truncated_longer = truncate_text(text, 4);
    assert_eq!(truncated_longer, "👋...");
}
