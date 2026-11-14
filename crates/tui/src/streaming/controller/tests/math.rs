// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Math while it streams: an open display is held as the tail and shown as
//! its source, never committed until it closes; whatever the chunks, the
//! rows committed equal the full render, with math on or off. Codex's
//! controller tests (their rich halves: this interface has no raw stream)
//! and samples of inline and display math, prices and shell text among
//! them.

use hanafubuki_agent::AgentTuiRendering;

use super::equality::{arbitrary_chunks, by_char, stream};
use super::full_render;
use crate::markdown_render::ListSpacing;
use crate::streaming::controller::TestStream;

const SAMPLES: &[&str] = &[
    "Before.\n\n\\[\n\\begin{aligned}\n\\underset{x}{\\min}&=\\sum_{i=1}^N x_{gpt}\\\\[2mm]\ny&=2\n\\end{aligned}\n\\]\n\nAfter $\\alpha$.\n",
    "Inline $\\alpha^2 + \\beta_{10}$ and \\(\\sqrt{x}\\), then $$\\frac{a}{b}$$ prose.\n",
    "Before.\n\n$$\n\\frac{a+b}{c}\n$$\n\nAfter $\\alpha^2$ with 中文 $x_1$ 和 \\(y^2\\)。\n",
    "$$\nx^2\n\n+y^2\n$$\n\nAfter.\n",
    "- Integral:\n\n  $$\\int_0^1 x\\,dx$$\n\n> Quoted:\n$$\\int_0^1 x\\,dx$$\n",
    "| Name | Value |\n|---|---|\n| **a** | $x^2$ |\n| b | \\(\\alpha\\) |\n\nAfter the table.\n",
    "Prices: $5 and $10; shell: $HOME/$USER, $(pwd), echo $$.\n\nMore $5.\n",
    "$$\nx\n$$ trailing\n$$\n\nAfter $\\alpha$.\n",
    "$$\nA=\\begin{pmatrix}1&2\\\\3&4\\end{pmatrix}\n$$\n\nDone.\n",
    "Unsupported: $\\unknown{x}$ and \\(\\frac{a}{b}^2\\).\n",
    "$$\n# heading\none\ntwo\nthree\nfour\nfive\n```rust\nx\n```\n$$\n\nAfter.\n",
    "Intro.\n\n$$ is the PID; `echo $$`.\n\n- item $x_1$\n",
];

/// Every sample, however it arrives, at every width: the committed rows
/// equal the full render and none is repaired.
fn assert_samples_stream_to_their_full_render() {
    for source in SAMPLES {
        for width in [None, Some(80), Some(40), Some(12)] {
            let expected = full_render(source, width);
            let splits: Vec<(&str, Vec<&str>)> = vec![
                ("lines", source.split_inclusive('\n').collect()),
                ("chars", by_char(source)),
                ("chunks 2", arbitrary_chunks(source, 2)),
                ("chunks 9", arbitrary_chunks(source, 9)),
                ("whole", vec![source]),
            ];
            for (split, chunks) in splits {
                let (streamed, diverged) = stream(&chunks, width, ListSpacing::AfterMultiline);
                assert_eq!(streamed, expected, "{split} at {width:?}:\n{source}");
                assert!(!diverged, "{split} at {width:?}:\n{source}");
            }
        }
    }
}

#[test]
fn math_streams_to_its_full_render_however_it_is_split() {
    assert_samples_stream_to_their_full_render();
}

#[test]
fn math_off_streams_to_its_full_render_too() {
    crate::markdown_render::preferences::set(AgentTuiRendering {
        math: false,
        ..Default::default()
    });
    assert_samples_stream_to_their_full_render();
}

#[test]
fn unicode_math_aligned_stream_matches_complete_render() {
    let source = SAMPLES[0];
    for width in [80, 12] {
        let (streamed, diverged) = stream(&by_char(source), Some(width), ListSpacing::default());
        assert_eq!(streamed, full_render(source, Some(width)));
        assert!(!diverged);
    }
}

#[test]
fn unicode_math_stream_holds_display_until_closed_and_preserves_source() {
    for ((open, close), prefix) in [(("$$", "$$"), "Before.\n\n"), (("\\[", "\\]"), "Before.\n")] {
        let mut stream = TestStream::new(Some(60), "/repo");
        let mut emitted = Vec::new();
        let mut source = String::new();
        for chunk in [
            prefix,
            open,
            "\n",
            "\\frac{a+b}{c}\n",
            "\n",
            close,
            "\n\n",
            "After $\\alpha^2$.\n",
        ] {
            source.push_str(chunk);
            stream.push(chunk);
            emitted.extend(stream.commit());
            if source.ends_with("\\frac{a+b}{c}\n") {
                assert_eq!(
                    super::lines_to_plain_strings(&emitted),
                    full_render(prefix, Some(60))
                );
                // The formula shows as it was written meanwhile.
                let tail = super::lines_to_plain_strings(&stream.tail());
                assert!(tail.contains(&"\\frac{a+b}{c}".to_string()), "{tail:?}");
            }
        }
        let (remaining, diverged) = stream.finalize_checked();
        emitted.extend(remaining);
        assert!(!diverged);
        assert_eq!(
            super::lines_to_plain_strings(&emitted),
            full_render(&source, Some(60))
        );
    }
}

#[test]
fn unicode_math_unfinished_display_is_visible_and_reflows() {
    for (open, close) in [("$$", "$$"), (r"\[", r"\]")] {
        let mut stream = TestStream::new(Some(80), "/repo");
        for (chunk, expected) in [(&open[..1], &open[..1]), (&open[1..], open)] {
            stream.push(chunk);
            assert_eq!(stream.tail()[0].to_string(), expected);
        }
        stream.push("\n \\frac{|a+b+c+d+e+f|}{g+h}");
        for width in [80, 20, 80] {
            stream.set_width(Some(width));
            let lines = stream.tail();
            assert!(lines.iter().any(|line| line.to_string().contains("frac")));
            assert!(lines.iter().all(|line| line.width() <= width));
            if open == "$$" && width == 20 {
                insta::assert_snapshot!(
                    "unicode_math_live_source_narrow",
                    lines
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
        stream.push(&format!("\n{close}\n"));
        let source = format!("{open}\n \\frac{{|a+b+c+d+e+f|}}{{g+h}}\n{close}\n");
        let (lines, diverged) = stream.finalize_checked();
        assert!(!diverged);
        assert_eq!(
            super::lines_to_plain_strings(&lines),
            full_render(&source, Some(80))
        );
    }
}

/// An answer that ends, or is interrupted, inside a display: the formula's
/// source stays, nothing committed is changed.
#[test]
fn an_unterminated_display_ends_as_its_source() {
    for ending in ["\n", ""] {
        for open in ["$$", "\\["] {
            let source = format!("Intro.\n\n{open}\n\\frac{{a+b+c+d+e+f}}{{g+h}}{ending}");
            for width in [Some(80), Some(20)] {
                let mut stream = TestStream::new(width, "/repo");
                let mut emitted = Vec::new();
                for chunk in by_char(&source) {
                    stream.push(chunk);
                    emitted.extend(stream.commit());
                }
                assert_eq!(
                    super::lines_to_plain_strings(&emitted),
                    vec!["Intro.".to_string()]
                );
                let (remaining, diverged) = stream.finalize_checked();
                emitted.extend(remaining);
                assert!(!diverged);
                let emitted = super::lines_to_plain_strings(&emitted);
                assert_eq!(emitted, full_render(&source, width));
                assert!(
                    emitted.iter().any(|line| line.starts_with(open)),
                    "{emitted:?}"
                );
            }
        }
    }
}

/// The line arriving inside an open display previews as its source,
/// wrapped, its TeX never read as Markdown (`\*` stays, `*x*` is not
/// italic), and a closed inline formula on the line previews rendered.
#[test]
fn the_preview_inside_a_display_is_its_source() {
    let mut stream = TestStream::new(Some(80), "/repo");
    stream.push("$$\n");
    stream.push("a \\* *b* [c](d");
    assert_eq!(
        super::lines_to_plain_strings(&stream.tail()),
        vec!["$$".to_string(), "a \\* *b* [c](d".to_string()]
    );
    let mut stream = TestStream::new(Some(80), "/repo");
    stream.push("Then $\\alpha^2$ and $5");
    assert_eq!(
        super::lines_to_plain_strings(&stream.tail()),
        vec!["Then α² and $5".to_string()]
    );
}

/// A line that opens a display previews as its source until its newline,
/// as in Codex, even once its closer arrived: then it is rendered.
#[test]
fn a_one_line_display_previews_as_source_until_its_newline() {
    let mut stream = TestStream::new(Some(80), "/repo");
    for chunk in ["$$\\frac{1}{2}", "$$"] {
        stream.push(chunk);
    }
    assert_eq!(
        super::lines_to_plain_strings(&stream.tail()),
        vec!["$$\\frac{1}{2}$$".to_string()]
    );
    stream.push("\n");
    let mut emitted = stream.commit();
    let (remaining, diverged) = stream.finalize_checked();
    emitted.extend(remaining);
    assert!(!diverged);
    assert_eq!(
        super::lines_to_plain_strings(&emitted),
        vec!["1".to_string(), "─".to_string(), "2".to_string()]
    );
}

/// After a ```` ```md ```` fence the answer unwraps, the open display's
/// start is the source's: the rows before it are committed while it is
/// open, and the stream still ends as the full render.
#[test]
fn an_open_display_after_an_unwrapped_fence_holds_only_itself() {
    let source =
        "```md\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n\nThen:\n\n$$\n\\frac{a}{b}\n$$\n\nDone.\n";
    let open = source.find("\\frac").unwrap();
    let mut stream = TestStream::new(Some(40), "/repo");
    stream.push(&source[..open]);
    let committed = super::lines_to_plain_strings(&stream.commit());
    assert_eq!(
        committed.last().map(String::as_str),
        Some("Then:"),
        "{committed:?}"
    );
    stream.push(&source[open..]);
    let mut emitted = committed;
    emitted.extend(super::lines_to_plain_strings(&stream.commit()));
    let (remaining, diverged) = stream.finalize_checked();
    emitted.extend(super::lines_to_plain_strings(&remaining));
    assert!(!diverged);
    assert_eq!(emitted, full_render(source, Some(40)));
}

/// Codex's limit: a `$$` after text pairs with a later closer within its
/// lookahead, and a formula between them is no longer math once that
/// closer arrives. A formula already committed then differs from the full
/// render, and the stream asks for Scrollback's repair at its end.
#[test]
fn a_late_closer_for_a_rejected_opener_asks_for_a_repair() {
    let source = "Run echo $$ here.\n\nThen $\\alpha$ there.\n\nAnd echo $$ again.\n";
    let chunks: Vec<&str> = source.split_inclusive('\n').collect();
    let (streamed, diverged) = stream(&chunks, Some(80), ListSpacing::AfterMultiline);
    assert!(diverged);
    assert!(
        streamed.contains(&"Then α there.".to_string()),
        "{streamed:?}"
    );
    assert!(full_render(source, Some(80)).contains(&"Then $\\alpha$ there.".to_string()));
}
