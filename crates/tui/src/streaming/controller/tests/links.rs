// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A stream's links: the rows it commits carry the links of the full
//! render's rows, however the source arrives and however links show; the
//! comparison at the end sees links, so a committed row that differs from
//! the final render only by its link is repaired too.

use super::stream_controller;
use crate::markdown_render::{preferences, ListSpacing, WebLinkDisplay};
use crate::streaming::controller::TestStream;
use crate::terminal_hyperlinks::HyperlinkLine;

const LINKED: &[&str] = &[
    "See [the documentation](https://example.com/docs) and [`code`](https://example.com/code).\n",
    "A bare https://example.com/a/very/long/path/that/wraps and (https://example.com/b).\n",
    "| Resource | Notes |\n|---|---|\n| [docs](https://example.com/docs) | https://example.com/plain |\n\nAfter.\n",
    "- [one](https://example.com/1) with enough words to wrap\n- https://example.com/2\n  - [nested **bold**](https://example.com/3)\n",
    "> Quoted [label](https://example.com/quoted) and 界ｶﾞ [wide](https://example.com/wide).\n",
    "1. [empty]() and [](https://example.com/empty) and [mail](mailto:a@example.com)\n",
];

fn full_render(source: &str, width: Option<usize>) -> Vec<HyperlinkLine> {
    crate::markdown::render_markdown_agent_with_list_spacing(
        source,
        width,
        Some("/repo"),
        ListSpacing::AfterMultiline,
    )
}

fn stream_linked(chunks: &[&str], width: Option<usize>) -> (Vec<HyperlinkLine>, bool) {
    let mut stream = TestStream::new(width, "/repo");
    let mut rows = Vec::new();
    for chunk in chunks {
        stream.push(chunk);
        rows.extend(stream.commit_linked());
    }
    let (remaining, diverged) = stream.finalize_linked_checked();
    rows.extend(remaining);
    (rows, diverged)
}

#[test]
fn a_streamed_answer_commits_the_links_of_its_full_render() {
    for display in [WebLinkDisplay::WithDestination, WebLinkDisplay::LabelOnly] {
        preferences::set_web_link_display(display);
        for source in LINKED {
            for width in [None, Some(80), Some(40), Some(20)] {
                let expected = full_render(source, width);
                assert!(expected.iter().any(|row| !row.hyperlinks.is_empty()));
                let by_char: Vec<&str> = source
                    .char_indices()
                    .map(|(index, ch)| &source[index..index + ch.len_utf8()])
                    .collect();
                for chunks in [source.split_inclusive('\n').collect(), by_char] {
                    let (streamed, diverged) = stream_linked(&chunks, width);
                    assert_eq!(streamed, expected, "{display:?} at {width:?}:\n{source}");
                    assert!(!diverged, "{display:?} at {width:?}:\n{source}");
                }
            }
        }
    }
    preferences::set_web_link_display(WebLinkDisplay::WithDestination);
}

#[test]
fn a_committed_row_differing_only_by_its_link_asks_for_a_repair() {
    let source = "See [docs](https://example.com/a).\n\nNext paragraph.\n";
    let mut stream = TestStream::new(Some(40), "/repo");
    stream.push(source);
    let committed = stream.commit_linked();
    assert_eq!(
        committed[0].hyperlinks[0].destination,
        "https://example.com/a"
    );
    let (_, diverged) = stream.finalize_linked_checked();
    assert!(!diverged);

    let mut stream = TestStream::new(Some(40), "/repo");
    stream.push(source);
    stream.commit_linked();
    stream.core.emitted[0].hyperlinks[0].destination = "https://example.com/b".into();
    let (_, diverged) = stream.finalize_linked_checked();
    assert!(diverged, "the row's text is the same, its link is not");
}

#[test]
fn the_tail_links_what_has_arrived_and_withholds_an_unfinished_destination() {
    let mut controller = stream_controller(Some(40));
    controller.push("See [docs](https://example.com/do");
    assert!(controller
        .tail_lines()
        .iter()
        .all(|row| row.hyperlinks.is_empty()));
    controller.push("cs) and https://example.com/bare more");
    let links: Vec<_> = controller
        .tail_lines()
        .iter()
        .flat_map(|row| row.hyperlinks.iter().map(|link| link.destination.clone()))
        .collect();
    assert!(
        links.contains(&"https://example.com/docs".to_string()),
        "{links:?}"
    );
    assert!(
        links.contains(&"https://example.com/bare".to_string()),
        "{links:?}"
    );
}
