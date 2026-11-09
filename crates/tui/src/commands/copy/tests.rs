// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

#[test]
fn copy_targets_keep_a_fenced_blocks_language_and_exact_content() {
    let markdown =
        "```python title=example\nprint('hi')\n```\n\n    ignored()\n\n~~~\nplain()\n~~~";
    assert_eq!(
        extract_copy_targets(markdown),
        vec![
            CopyTarget::Code {
                language: Some("python".to_string()),
                content: "print('hi')\n".to_string(),
            },
            CopyTarget::Code {
                language: None,
                content: "plain()\n".to_string(),
            },
        ]
    );
    assert_eq!(
        extract_copy_targets("```powershell\r\nGet-Item .\r\nWrite-Output ok\r\n```\r\n"),
        vec![CopyTarget::Code {
            language: Some("powershell".to_string()),
            content: "Get-Item .\r\nWrite-Output ok\r\n".to_string(),
        }]
    );
}

#[test]
fn copy_targets_keep_a_nested_quotes_markdown_in_source_order() {
    let markdown = "> outer **bold**\n> > inner *quote*\n> ```sh\n> nested()\n> ```\n";
    assert_eq!(
        extract_copy_targets(markdown),
        vec![
            CopyTarget::Quote(
                "outer **bold**\n> inner *quote*\n```sh\nnested()\n```\n".to_string()
            ),
            CopyTarget::Code {
                language: Some("sh".to_string()),
                content: "nested()\n".to_string(),
            },
        ]
    );
}

#[test]
fn a_quote_without_prose_is_left_out() {
    let markdown = "> ```sh\n> code()\n> ```\n\n> quoted prose\n";
    assert_eq!(
        extract_copy_targets(markdown),
        vec![
            CopyTarget::Code {
                language: Some("sh".to_string()),
                content: "code()\n".to_string(),
            },
            CopyTarget::Quote("quoted prose\n".to_string()),
        ]
    );
}

#[test]
fn the_choices_are_the_whole_answer_then_its_parts() {
    let answer = "Run this:\n\n```bash\nls -la\n```\n\n> Mind the dot files.\n\n```\nplain\n```\n";
    let choices = copy_choices(answer);
    let labels: Vec<&str> = choices.iter().map(|(label, _)| label.as_str()).collect();
    assert_eq!(labels, ["Whole answer", "bash code", "Quote", "Code block"]);
    assert_eq!(choices[0].1, answer);
    assert_eq!(choices[1].1, "ls -la\n");
    assert_eq!(choices[2].1, "Mind the dot files.\n");
    assert!(copy_choices("  \n").is_empty());
}
