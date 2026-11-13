// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Web links keep their destination after a link-styled label; local file
//! links show their target, relative to the session's directory.

use insta::assert_snapshot;
use ratatui::style::Stylize;
use ratatui::text::{Line, Text};

use super::plain_lines;
use crate::markdown_render::{
    render_markdown_text, render_markdown_text_with_width, render_markdown_text_with_width_and_cwd,
};

fn render_for_cwd(input: &str, cwd: &str) -> Text<'static> {
    render_markdown_text_with_width_and_cwd(input, None, Some(cwd))
}

#[test]
fn web_link_labels_use_link_style_and_preserve_inline_formatting() {
    for (label, expected_label) in [
        ("plain", "plain".cyan().underlined()),
        ("`code`", "code".cyan().underlined()),
        ("**bold**", "bold".cyan().bold().underlined()),
        ("*italic*", "italic".cyan().italic().underlined()),
    ] {
        let text = render_markdown_text(&format!(
            "before [{label}](https://example.com) after `code`"
        ));
        let expected = Text::from(Line::from_iter([
            "before ".into(),
            expected_label,
            " (".into(),
            "https://example.com".cyan().underlined(),
            ")".into(),
            " after ".into(),
            "code".cyan(),
        ]));
        assert_eq!(text, expected, "label: {label}");
    }
}

#[test]
fn web_link_labels_keep_link_style_in_wrapped_prose_and_tables() {
    let links =
        "[plain](https://example.com) [`code`](https://example.com) [<b>](https://example.com)";
    for markdown in [
        links.to_string(),
        format!("| Links |\n| --- |\n| {links} |"),
    ] {
        for width in [32, 80] {
            let text = render_markdown_text_with_width(&markdown, Some(width));
            let labels = text
                .lines
                .iter()
                .flat_map(|line| &line.spans)
                .filter(|span| matches!(span.content.as_ref(), "plain" | "code" | "<b>"))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                labels,
                vec![
                    "plain".cyan().underlined(),
                    "code".cyan().underlined(),
                    "<b>".cyan().underlined()
                ],
                "{markdown} at {width}"
            );
        }
    }
}

#[test]
fn non_web_links_keep_a_plain_label() {
    let text = render_markdown_text("[mail](mailto:test@example.com)");
    assert_eq!(
        text,
        Text::from(Line::from_iter([
            "mail".into(),
            " (".into(),
            "mailto:test@example.com".cyan().underlined(),
            ")".into(),
        ]))
    );
}

#[test]
fn file_link_preserves_label_and_decodes_fallback_destination() {
    let text = render_for_cwd(
        "[/Users/example/code/codex/codex-rs/tui/src/My%20File.rs](/Users/example/code/codex/codex-rs/tui/src/My%20File.rs)",
        "/Users/example/code/codex",
    );
    let expected = Text::from(Line::from_iter([
        "/Users/example/code/codex/codex-rs/tui/src/My%20File.rs".into(),
        " (".into(),
        "codex-rs/tui/src/My File.rs".cyan(),
        ")".into(),
    ]));
    assert_eq!(text, expected);
}

#[test]
fn file_link_keeps_descriptive_label_and_target() {
    let text = render_for_cwd(
        "Your `codex` launcher [automatically adds those overrides](/home/dev-user/code/openai/project/dotslash-gen/bin/codex:1105), even though you did not specify any.",
        "/home/dev-user/code/openai",
    );
    let expected = Text::from(Line::from_iter([
        "Your ".into(),
        "codex".cyan(),
        " launcher ".into(),
        "automatically adds those overrides".into(),
        " (".into(),
        "project/dotslash-gen/bin/codex:1105".cyan(),
        ")".into(),
        ", even though you did not specify any.".into(),
    ]));
    assert_eq!(text, expected);
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn file_link_preserves_tilde_and_absolute_destinations() {
    let markdown = "[~/notes](~/notes)\n\n\
         [/home/alice/notes](/home/alice/notes)\n\n\
         [/home/alice/notes](~/notes)\n\n\
         [~/notes](/home/alice/notes)\n\n\
         [my **notes**](~/notes#L12C3)\n\n\
         [~/project/src/lib.rs](~/project/src/lib.rs)\n\n\
         [/home/alice/project/src/lib.rs](/home/alice/project/src/lib.rs)\n\n\
         [~](/home/alice:12)";
    let text = render_for_cwd(markdown, "/home/alice/project");
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn file_link_preserves_path_labels_and_formats_destinations() {
    let markdown = r"[file:///repo/src/lib.rs](file:///repo/src/lib.rs)

[SRC\LIB.RS](/repo/src/lib.rs#L12)

[./src/lib.rs](/repo/src/lib.rs)

[file:///C:/Repo/Src/Lib.rs](C:/Repo/Src/Lib.rs)

[FILE:///C:/Repo/Src/Lib.rs](C:/Repo/Src/Lib.rs)

[file://server/share/My%20File.rs](//server/share/My%20File.rs)

[//SERVER/SHARE/My File.rs](file://server/share/My%20File.rs)

[file:///repo/My%20File.rs](/repo/My%20File.rs)

[file:///repo/percent%2520.rs](/repo/percent%2520.rs)

[percent%20.rs](/repo/percent%2520.rs)

[open **My File.rs**](/repo/My%20File.rs)

[other/src/lib.rs](/repo/src/lib.rs)";
    let text = render_for_cwd(markdown, "/repo");
    // Codex keeps the escapes of a UNC file URL on Unix only; here the
    // display is the same everywhere, but the snapshot stays Codex's.
    let rendered = plain_lines(&text)
        .join("\n")
        .replace("My%20File.rs", "My File.rs");
    assert_snapshot!(rendered);
}

#[test]
fn file_link_decodes_percent_encoded_bare_path_destination() {
    let text = render_for_cwd(
        "[open Example Folder/Résumé/report.md](/Users/example/code/codex/Example%20Folder/R%C3%A9sum%C3%A9/report.md)",
        "/Users/example/code/codex",
    );
    let expected = Text::from(Line::from_iter([
        "open Example Folder/Résumé/report.md".into(),
        " (".into(),
        "Example Folder/Résumé/report.md".cyan(),
        ")".into(),
    ]));
    assert_eq!(text, expected);
}

#[test]
fn file_link_preserves_labels_with_invalid_percent_encoding() {
    let text = render_for_cwd("[bad%FF label](/tmp/)\n\n[bad%FF label](/)", "/repo");
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn file_link_preserves_directory_labels_and_destinations() {
    let text = render_for_cwd(
        "[dir](./dir/)\n\n[dir/](./dir)\n\n[dir](/outside/dir/)\n\n[dir/](/outside/dir)",
        "/repo",
    );
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn file_link_keeps_unrelated_relative_label_with_matching_suffix() {
    let text = render_for_cwd("[other/src/lib.rs](/repo/src/lib.rs)", "/repo");
    let expected = Text::from(Line::from_iter([
        "other/src/lib.rs".into(),
        " (".into(),
        "src/lib.rs".cyan(),
        ")".into(),
    ]));
    assert_eq!(text, expected);
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn file_link_locations_snapshot() {
    let cwd = "/Users/example/code/codex";
    let file = "/Users/example/code/codex/codex-rs/tui/src/markdown_render.rs";
    let rendered = [
        format!("[markdown_render.rs]({file}:74)"),
        format!("[markdown_render.rs](file://{file}#L74C3)"),
        format!("[markdown_render.rs#L74C3](file://{file}#L74C3)"),
        format!("[markdown_render.rs]({file}:74:3-76:9)"),
        format!("[markdown_render.rs:74:3-76:9]({file}:74:3-76:9)"),
        format!("[markdown_render.rs](file://{file}#L74C3-L76C9)"),
        format!("[markdown_render.rs#L74C3-L76C9](file://{file}#L74C3-L76C9)"),
        format!("[markdown_render.rs]({file}:74–76)"),
        "[README.md:74](/Users/example/code/codex/README.md:74)".to_string(),
    ]
    .map(|source| plain_lines(&render_for_cwd(&source, cwd)).join("\n"))
    .join("\n");
    assert_snapshot!(rendered, @r"
    markdown_render.rs (codex-rs/tui/src/markdown_render.rs:74)
    markdown_render.rs (codex-rs/tui/src/markdown_render.rs:74:3)
    markdown_render.rs#L74C3 (codex-rs/tui/src/markdown_render.rs:74:3)
    markdown_render.rs (codex-rs/tui/src/markdown_render.rs:74:3-76:9)
    markdown_render.rs:74:3-76:9 (codex-rs/tui/src/markdown_render.rs:74:3-76:9)
    markdown_render.rs (codex-rs/tui/src/markdown_render.rs:74:3-76:9)
    markdown_render.rs#L74C3-L76C9 (codex-rs/tui/src/markdown_render.rs:74:3-76:9)
    markdown_render.rs (codex-rs/tui/src/markdown_render.rs:74–76)
    README.md:74 (README.md:74)
    ");
}

#[test]
fn file_links_preserve_foreign_windows_paths_and_anchors_snapshot() {
    let rendered = [
        ("file:///C:/repo/Quarterly%20Report.xlsx", "C:/repo"),
        (
            "file://server/share/Quarterly%20Report.xlsx",
            "//server/share",
        ),
        ("C:\\repo\\src\\main.rs:3", "C:\\repo"),
    ]
    .map(|(destination, cwd)| {
        render_for_cwd(&format!("[report]({destination}#L4C2)"), cwd).to_string()
    })
    .join("\n");
    insta::assert_snapshot!(rendered, @r"
    report (Quarterly Report.xlsx:4:2)
    report (Quarterly Report.xlsx:4:2)
    report (src/main.rs:3:4:2)
    ");
}

#[test]
fn multiline_file_link_label_after_styled_prefix_does_not_panic() {
    let text = render_for_cwd(
        "**bold** plain [foo\nbar](file:///Users/example/code/codex/codex-rs/tui/src/markdown_render.rs#L74C3)",
        "/Users/example/code/codex",
    );
    let expected = Text::from(Line::from_iter([
        "bold".bold(),
        " plain ".into(),
        "foo".into(),
        " ".into(),
        "bar".into(),
        " (".into(),
        "codex-rs/tui/src/markdown_render.rs:74:3".cyan(),
        ")".into(),
    ]));
    assert_eq!(text, expected);
}

#[test]
fn markdown_render_file_link_snapshot() {
    let markdown = "[analysis.md](/tmp/codex-link-repro/analysis/021_all_experiments_best_per_group/analysis.md)\n\n\
        [**analysis** `.md`](/tmp/codex-link-repro/analysis.md:12)\n\n\
        [](/tmp/codex-link-repro/analysis.md#L12C3)\n\n\
        [   ](../analysis.md)\n\n\
        [](<file://[>)\n\n\
        [broken](<file://[>)\n\n\
        | File |\n| --- |\n| [analysis.md](/tmp/codex-link-repro/analysis.md) |\n| [](./empty.md) |";
    let text = render_for_cwd(markdown, "/tmp/codex-link-repro");
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn unordered_list_local_file_link_stays_inline_with_following_text() {
    let text = render_markdown_text_with_width_and_cwd(
        "- [binary](/Users/example/code/codex/codex-rs/README.md:93): core is the agent/business logic, tui is the terminal UI, exec is the headless automation surface, and cli is the top-level multitool binary.",
        Some(72),
        Some("/Users/example/code/codex"),
    );
    assert_eq!(
        plain_lines(&text),
        vec![
            "• binary (codex-rs/README.md:93): core is the agent/business logic, tui",
            "  is the terminal UI, exec is the headless automation surface, and cli",
            "  is the top-level multitool binary.",
        ]
    );
}

/// Without a working directory an absolute target stays absolute.
#[test]
fn file_link_without_a_directory_keeps_the_absolute_target() {
    let text = render_markdown_text_with_width_and_cwd("[lib](/repo/src/lib.rs)", None, None);
    assert_eq!(plain_lines(&text), vec!["lib (/repo/src/lib.rs)"]);
}

#[test]
fn mixed_url_markdown_wraps_prose_without_splitting_words_snapshot() {
    let md = "This paragraph keeps **strikethrough** intact near a [link](https://example.com/path) while enough surrounding prose forces wrapping.";
    let text = render_markdown_text_with_width(md, Some(48));
    assert_snapshot!(plain_lines(&text).join("\n"));
}
