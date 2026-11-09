use super::form::{join_arguments, split_arguments};

#[test]
fn arguments_split_at_spaces_and_keep_quoted_parts_whole() {
    assert_eq!(
        split_arguments(r#"-y  @scope/server --name "two words" 'it''s' "a \"q\"""#),
        [
            "-y",
            "@scope/server",
            "--name",
            "two words",
            "its",
            r#"a "q""#
        ]
    );
    assert_eq!(split_arguments(r#"--empty "" end"#), ["--empty", "", "end"]);
    assert!(split_arguments("   ").is_empty());
}

#[test]
fn joined_arguments_read_back_the_same() {
    let arguments: Vec<String> = ["-c", "exit 0", "", r#"say "hi""#, r"C:\tools", "plain"]
        .map(String::from)
        .into();
    let line = join_arguments(&arguments);
    assert_eq!(line, r#"-c "exit 0" "" "say \"hi\"" "C:\\tools" plain"#);
    assert_eq!(split_arguments(&line), arguments);
}
