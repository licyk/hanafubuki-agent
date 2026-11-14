// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Codex's web-link tests: which terminals show a label alone, what the
//! label-only and the fallback presentations render, bare URLs styled and
//! linked, and the links kept through prose and table wrapping and through
//! a stream. The terminal is injected, never the runner's.

use std::collections::{BTreeSet, HashMap};

use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

use super::*;
use crate::markdown_render::{preferences, render_markdown_lines, ListSpacing, MarkdownStyles};
use crate::terminal_detection::{detect, Multiplexer};
use crate::terminal_hyperlinks::{visible_lines, TerminalHyperlink};
use crate::wrapping::display_width;

fn link_style() -> Style {
    MarkdownStyles::default().link
}

fn render(markdown: &str, width: usize, display: WebLinkDisplay) -> Vec<HyperlinkLine> {
    preferences::set_web_link_display(display);
    let lines = render_markdown_lines(markdown, Some(width), None, ListSpacing::AfterMultiline);
    preferences::set_web_link_display(WebLinkDisplay::WithDestination);
    lines
}

fn texts(lines: &[HyperlinkLine]) -> Vec<String> {
    lines.iter().map(|line| line.line.to_string()).collect()
}

fn terminal(name: TerminalName) -> TerminalInfo {
    TerminalInfo {
        name,
        multiplexer: None,
        term: None,
    }
}

#[test]
fn supporting_terminals_render_only_the_styled_label_and_keep_its_target() {
    for name in [
        TerminalName::Ghostty,
        TerminalName::Iterm2,
        TerminalName::WezTerm,
        TerminalName::Rex,
        TerminalName::Kitty,
        TerminalName::VsCode,
        TerminalName::Alacritty,
        TerminalName::WindowsTerminal,
        TerminalName::Konsole,
        TerminalName::GnomeTerminal,
        TerminalName::Vte,
    ] {
        let display = WebLinkDisplay::for_terminal(&terminal(name), true, false);
        assert_eq!(display, WebLinkDisplay::LabelOnly, "{name:?}");
        for (markdown, label) in [
            (
                "[label](https://example.com)",
                Span::styled("label", link_style()),
            ),
            (
                "[`label`](https://example.com)",
                Span::styled("label", link_style()),
            ),
            (
                "[**label**](https://example.com)",
                Span::styled("label", link_style()).bold(),
            ),
            (
                "[*label*](https://example.com)",
                Span::styled("label", link_style()).italic(),
            ),
            (
                "[<b>](https://example.com)",
                Span::styled("<b>", link_style()),
            ),
            (
                "[https://example.com](https://example.com)",
                Span::styled("https://example.com", link_style()),
            ),
        ] {
            let label_width = label.width();
            assert_eq!(
                render(markdown, 80, display),
                vec![HyperlinkLine {
                    line: Line::from(label),
                    hyperlinks: vec![TerminalHyperlink {
                        columns: 0..label_width,
                        destination: "https://example.com".into(),
                    }],
                }],
                "terminal: {name:?}, {markdown}"
            );
        }
    }
}

#[test]
fn unknown_terminals_multiplexers_and_pipes_keep_visible_destinations() {
    let mut tmux = terminal(TerminalName::Ghostty);
    tmux.multiplexer = Some(Multiplexer::Tmux);
    let mut zellij = terminal(TerminalName::Ghostty);
    zellij.multiplexer = Some(Multiplexer::Zellij);
    let with_term = |term: &str| TerminalInfo {
        term: Some(term.into()),
        ..terminal(TerminalName::Ghostty)
    };
    for (info, is_terminal, screen) in [
        (terminal(TerminalName::Unknown), true, false),
        (terminal(TerminalName::AppleTerminal), true, false),
        (terminal(TerminalName::WarpTerminal), true, false),
        (terminal(TerminalName::Dumb), true, false),
        (tmux, true, false),
        (zellij, true, false),
        (with_term("screen-256color"), true, false),
        (with_term("tmux-256color"), true, false),
        (with_term("dumb"), true, false),
        (terminal(TerminalName::Ghostty), false, false),
        (terminal(TerminalName::Ghostty), true, true),
    ] {
        assert_eq!(
            WebLinkDisplay::for_terminal(&info, is_terminal, screen),
            WebLinkDisplay::WithDestination,
            "{info:?} {is_terminal} {screen}"
        );
    }
}

#[test]
fn the_display_follows_the_environment_of_each_platform() {
    let display = |vars: &[(&str, &str)]| {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        WebLinkDisplay::for_terminal(&detect(|name| vars.get(name).cloned()), true, false)
    };
    use WebLinkDisplay::{LabelOnly, WithDestination};
    let cases: &[(&[(&str, &str)], WebLinkDisplay)] = &[
        // Windows: Windows Terminal, the legacy console, VS Code.
        (&[("WT_SESSION", "0b9c")], LabelOnly),
        (&[], WithDestination),
        (
            &[("TERM_PROGRAM", "vscode"), ("WT_SESSION", "1")],
            LabelOnly,
        ),
        // macOS: Terminal.app, iTerm2, Ghostty, WezTerm.
        (
            &[
                ("TERM_PROGRAM", "Apple_Terminal"),
                ("TERM", "xterm-256color"),
            ],
            WithDestination,
        ),
        (
            &[("TERM_PROGRAM", "iTerm.app"), ("TERM", "xterm-256color")],
            LabelOnly,
        ),
        (&[("TERM", "xterm-ghostty")], LabelOnly),
        (&[("WEZTERM_VERSION", "20240203")], LabelOnly),
        // Linux: a bare xterm, GNOME's VTE, Konsole, kitty, a CI runner.
        (&[("TERM", "xterm-256color")], WithDestination),
        (
            &[("VTE_VERSION", "7600"), ("TERM", "xterm-256color")],
            LabelOnly,
        ),
        (&[("KONSOLE_VERSION", "230800")], LabelOnly),
        (&[("TERM", "xterm-kitty")], LabelOnly),
        (&[("TERM", "dumb"), ("CI", "true")], WithDestination),
        // Multiplexers keep the destination, whatever runs them.
        (
            &[
                ("TERM_PROGRAM", "tmux"),
                ("TMUX", "/t,1,0"),
                ("WT_SESSION", "1"),
            ],
            WithDestination,
        ),
        (
            &[("ZELLIJ", "0"), ("TERM_PROGRAM", "WezTerm")],
            WithDestination,
        ),
        (
            &[("TERM", "screen-256color"), ("VTE_VERSION", "7600")],
            WithDestination,
        ),
    ];
    for (vars, expected) in cases {
        assert_eq!(display(vars), *expected, "{vars:?}");
    }
}

#[test]
fn label_only_policy_keeps_non_web_links_and_literal_urls_unchanged() {
    let markdown = "[file](./src/main.rs) [mail](mailto:a@example.com)\n\nhttps://example.com\n\n`https://example.com/code`";
    assert_eq!(
        render(markdown, 80, WebLinkDisplay::LabelOnly),
        render(markdown, 80, WebLinkDisplay::WithDestination),
    );
}

#[test]
fn empty_web_link_labels_keep_a_visible_destination() {
    for markdown in [
        "[](https://example.com)",
        "[ ](https://example.com)",
        "[\u{200d}](https://example.com)",
    ] {
        let label_only = render(markdown, 80, WebLinkDisplay::LabelOnly);
        assert_eq!(
            label_only,
            render(markdown, 80, WebLinkDisplay::WithDestination)
        );
        assert!(label_only[0]
            .line
            .to_string()
            .contains("(https://example.com)"));
    }
}

#[test]
fn the_fallback_links_the_label_and_the_destination() {
    let lines = render(
        "See [docs](https://example.com/docs).",
        80,
        WebLinkDisplay::WithDestination,
    );
    assert_eq!(texts(&lines), ["See docs (https://example.com/docs)."]);
    assert_eq!(
        lines[0].hyperlinks,
        [
            TerminalHyperlink {
                columns: 4..8,
                destination: "https://example.com/docs".into()
            },
            TerminalHyperlink {
                columns: 10..34,
                destination: "https://example.com/docs".into()
            },
        ]
    );
}

#[test]
fn label_only_web_links_keep_targets_after_prose_and_table_wrapping() {
    let links = "[plain label](https://example.com/plain) [`code label`](https://example.com/code) [<b>](https://example.com/html)";
    for markdown in [
        links.to_string(),
        format!("| Links |\n| --- |\n| {links} |"),
    ] {
        for width in [12, 40] {
            let lines = render(&markdown, width, WebLinkDisplay::LabelOnly);
            let targets = lines
                .iter()
                .flat_map(|line| &line.hyperlinks)
                .map(|link| link.destination.as_str())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                targets,
                BTreeSet::from([
                    "https://example.com/plain",
                    "https://example.com/code",
                    "https://example.com/html",
                ]),
            );
            assert!(texts(&lines).iter().all(|line| !line.contains("https://")));
        }
    }
}

#[test]
fn bare_urls_are_styled_without_coloring_surrounding_prose() {
    let lines = render(
        "界 (https://example.com/a); https://example.com/b! **https://example.com/c**",
        100,
        WebLinkDisplay::LabelOnly,
    );
    assert_eq!(
        visible_lines(lines.clone()),
        vec![Line::from(vec![
            "界 (".into(),
            Span::styled("https://example.com/a", link_style()),
            "); ".into(),
            Span::styled("https://example.com/b", link_style()),
            "! ".into(),
            Span::styled("https://example.com/c", link_style()).bold(),
        ])],
    );
    let destinations: Vec<_> = lines[0]
        .hyperlinks
        .iter()
        .map(|link| (link.columns.clone(), link.destination.as_str()))
        .collect();
    assert_eq!(
        destinations,
        [
            (4..25, "https://example.com/a"),
            (28..49, "https://example.com/b"),
            (51..72, "https://example.com/c"),
        ]
    );
}

#[test]
fn bare_url_styling_preserves_tabs_and_unicode() {
    let writer = super::super::Writer::new("", None, None, WebLinkDisplay::LabelOnly);
    let line = writer.style_bare_web_urls("界\t(https://example.com).".bold());
    assert_eq!(
        line.line,
        Line::from(vec![
            "界\t(".bold(),
            Span::styled(
                "https://example.com",
                Style::new().bold().patch(link_style())
            ),
            ").".bold(),
        ]),
    );
}

#[test]
fn bare_urls_keep_link_style_and_targets_when_wrapped_in_tables() {
    let lines = render(
        "| Resource |\n| --- |\n| https://example.com/docs |",
        20,
        WebLinkDisplay::LabelOnly,
    );
    let linked: String = lines
        .iter()
        .flat_map(|line| {
            let text = line.line.to_string();
            line.hyperlinks
                .iter()
                .map(move |link| {
                    assert_eq!(link.destination, "https://example.com/docs");
                    text.chars()
                        .skip(link.columns.start)
                        .take(link.columns.len())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(linked, "https://example.com/docs");
}

#[test]
fn streaming_and_full_render_agree_with_label_only_links() {
    let markdown = "See [**label**](https://example.com).\n\n| Resource |\n| --- |\n| [`code`](https://example.com/code) |\n\nTrailing text.";
    for width in [12, 40] {
        for end in 1..=markdown.len() {
            if !markdown.is_char_boundary(end) {
                continue;
            }
            let prefix = &markdown[..end];
            preferences::set_web_link_display(WebLinkDisplay::LabelOnly);
            let streamed = crate::markdown_render::render_streaming_markdown(
                prefix,
                Some(width),
                None,
                ListSpacing::AfterMultiline,
            );
            preferences::set_web_link_display(WebLinkDisplay::WithDestination);
            assert_eq!(
                streamed.lines,
                render(prefix, width, WebLinkDisplay::LabelOnly)
            );
        }
    }
}

#[test]
fn label_only_and_fallback_presentations_snapshot() {
    let markdown = "plain [plain](https://example.com) `code` [`code`](https://example.com)\n\n| Resource |\n| --- |\n| [docs](https://example.com/docs) |";
    let presentations = [WebLinkDisplay::LabelOnly, WebLinkDisplay::WithDestination]
        .map(|display| texts(&render(markdown, 40, display)).join("\n"));
    insta::assert_snapshot!(presentations.join("\n\n---\n\n"));
}

#[test]
fn a_render_without_hyperlinks_keeps_the_destination_on_any_terminal() {
    preferences::set_web_link_display(WebLinkDisplay::LabelOnly);
    let mut lines = Vec::new();
    crate::markdown::append_markdown(
        "See [docs](https://example.com/docs).",
        Some(80),
        None,
        &mut lines,
    );
    preferences::set_web_link_display(WebLinkDisplay::WithDestination);
    assert_eq!(
        lines.iter().map(Line::to_string).collect::<Vec<_>>(),
        ["See docs (https://example.com/docs)."]
    );
}

const SWEEP: &str = "> Quoted [label one](https://e.com/1) text\n\n- a\n  - [nested label](https://e.com/2) more words\n\n1. item\n\n   | A | B |\n   |---|---|\n   | [cell link](https://e.com/3) | x |\n\n| Name | Notes |\n|---|---|\n| [rec link](https://e.com/4) | A long narrative note that starves a narrow column of room. |\n| other | Another long narrative note for the second record here. |\n\nsee https://e.com/5/a/path and [漢字ｶﾞ label](https://e.com/6).\n";

/// The text under each destination's columns, its rows joined, spaces
/// left out (a wrap moves them).
fn under_links(lines: &[HyperlinkLine]) -> std::collections::BTreeMap<String, String> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut joined = std::collections::BTreeMap::<String, String>::new();
    for line in lines {
        let text: String = line
            .line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        for link in &line.hyperlinks {
            let mut column = 0;
            let under = joined.entry(link.destination.clone()).or_default();
            for grapheme in text.graphemes(true) {
                if link.columns.contains(&column) && !grapheme.trim().is_empty() {
                    under.push_str(grapheme);
                }
                column += display_width(grapheme);
            }
        }
    }
    joined
}

/// The same, from the cells a buffer was painted with.
fn under_linked_cells(
    buffer: &ratatui::buffer::Buffer,
) -> std::collections::BTreeMap<String, String> {
    let mut joined = std::collections::BTreeMap::<String, String>::new();
    for position in buffer.area.positions() {
        if let Some((destination, visible)) =
            crate::terminal_hyperlinks::osc8_parts(buffer[position].symbol())
        {
            if !visible.trim().is_empty() {
                joined
                    .entry(destination.to_string())
                    .or_default()
                    .push_str(visible);
            }
        }
    }
    joined
}

fn expected_under_links(display: WebLinkDisplay) -> std::collections::BTreeMap<String, String> {
    [
        ("https://e.com/1", "labelone"),
        ("https://e.com/2", "nestedlabel"),
        ("https://e.com/3", "celllink"),
        ("https://e.com/4", "reclink"),
        ("https://e.com/5/a/path", ""),
        ("https://e.com/6", "漢字ｶﾞlabel"),
    ]
    .into_iter()
    .map(|(destination, label)| {
        let shown = match (display, label) {
            (_, "") => destination.to_string(),
            (WebLinkDisplay::LabelOnly, label) => label.to_string(),
            (WebLinkDisplay::WithDestination, label) => format!("{label}{destination}"),
        };
        (destination.to_string(), shown)
    })
    .collect()
}

/// At every stage a link goes through — the render, the answer's and the
/// plan's cells, the transcript's rows laid out narrower, the buffer's
/// cells — the text under its columns is its label: no shift of a prefix,
/// a table's padding, a record's indent or a wide glyph moves it.
#[test]
fn the_text_under_each_link_is_its_label_at_every_stage() {
    use crate::history_cell::{AgentMarkdownCell, HistoryCell, ProposedPlanCell};
    use crate::transcript_view::TextLayout;
    for display in [WebLinkDisplay::LabelOnly, WebLinkDisplay::WithDestination] {
        preferences::set_web_link_display(display);
        let expected = expected_under_links(display);
        for width in [24u16, 30, 50, 80] {
            let what = format!("{display:?} at {width}");
            let rendered = render_markdown_lines(
                SWEEP,
                Some(usize::from(width)),
                None,
                ListSpacing::AfterMultiline,
            );
            assert_eq!(under_links(&rendered), expected, "render, {what}");
            let answer = AgentMarkdownCell::new(SWEEP.to_string(), "/repo".to_string());
            let plan = ProposedPlanCell::new(SWEEP.to_string(), "/repo".to_string());
            for lines in [
                answer.display_hyperlink_lines(width),
                answer.retained_hyperlink_lines(width, true),
                plan.display_hyperlink_lines(width),
            ] {
                assert_eq!(under_links(&lines), expected, "cell, {what}");
                let narrower = width - 7;
                let layout = TextLayout::linked(lines, narrower);
                let area = ratatui::layout::Rect::new(0, 0, narrower, layout.row_count() as u16);
                let mut buffer = ratatui::buffer::Buffer::empty(area);
                layout.render(area, &mut buffer, 0);
                assert_eq!(under_linked_cells(&buffer), expected, "buffer, {what}");
            }
        }
    }
    preferences::set_web_link_display(WebLinkDisplay::WithDestination);
}
