//! Dollar signs and TeX-like delimiters in ordinary text: prices, shell,
//! Makefiles, regular expressions, PHP, Perl and jQuery, escapes and
//! unbalanced delimiters, in prose, tables, lists and quotes. None is
//! recognised as math, so the parser reads the source as it did before
//! math (nothing masked, nothing replaced). The cases Codex's rules do
//! take as math are pinned apart, as the known limits.

use pulldown_cmark::Options;

use super::super::MathMarkdown;
use super::plain;
use crate::markdown_render::{render_markdown_lines, ListSpacing};

const ORDINARY: &[&str] = &[
    // Prices and currency.
    "It costs $5 and $10.",
    "Plans: $5.00, $10.00 and $15.00 a month.",
    "Between $5-$10 per seat, or $5/$10 with tax.",
    "From $1,000 to $2,000 ($1.5k on average).",
    "Salaries of $100k-$200k; ($5) or ($10).",
    "US$5 and US$10, or 5$ and 10$.",
    "$5 – $10, $9.99/mo, $99/yr. Save $5!",
    "Prices in $USD$, or in $EUR$.",
    "价格是$5和$10，费用$5，税$1。",
    // Shell.
    "Use $HOME, $HOME/$USER and $PATH:$HOME.",
    "Run $(git rev-parse HEAD) and ${HOME}/bin.",
    "Arguments $1 and $2, \"$1\" \"$2\", $@ and $*, $# of them.",
    "The status $? and $!, the PID $$ and $0.",
    "export PATH=$HOME/bin:$PATH and cd $OLDPWD",
    "kill $$ then echo $$ again",
    // Makefiles.
    "$(CC) $(CFLAGS) -o $@ $^ and $< with $(wildcard *.c).",
    "$@: $^ then $(MAKE) -C sub",
    // Regular expressions.
    "Match ^foo$ and ^bar$ or /^\\d+$/.",
    "Anchors: a$ and ^b, then $ alone.",
    // PHP, Perl, jQuery.
    "$x = 1; $y = 2; $this->foo and $_POST['x'].",
    "my $x = $y + 1; print $x; $foo->$bar; $a.$b",
    "$x=$y; $x==$y; ($x, $y); $_GET and $_POST",
    "$('#id').hide(); $.ajax(url); $(document).ready(fn)",
    // Escapes and unbalanced delimiters.
    "\\$5 and \\$10, \\$x\\$ and \\$\\alpha\\$.",
    "An escaped \\$x$ stays.",
    "Escaped \\\\(not math\\\\) and \\\\[not math\\\\].",
    "Open $\\alpha without a closer.",
    "Open \\(x without a closer.",
    "Text then $$ x without a closer.",
    "Text then \\[ x without a closer.",
    // Code.
    "`$x$` and `$$y$$` and `\\(z\\)`.",
    "```sh\necho $HOME $$ $(pwd)\nexport X=$5$\n```",
    "    $x^2$ in an indented block",
    // Tables, lists and quotes.
    "| Price | Shell |\n|---|---|\n| $5 and $10 | $HOME/$USER |\n| $(cmd) | ^a$ and ^b$ |\n",
    "- $5 and $10\n- $HOME and $PATH\n- ^foo$ and ^bar$\n  - $x = 1; $y = 2;\n",
    "1. Costs $5.\n2. Costs $10.\n",
    "> It costs $5 and $10.\n> Run $(cmd).\n",
];

fn masked(source: &str) -> MathMarkdown<'_> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    MathMarkdown::new(source, options, Some(80))
}

#[test]
fn ordinary_dollars_are_never_math() {
    for source in ORDINARY {
        let math = masked(source);
        assert!(math.replacements.is_empty(), "{source}");
        assert_eq!(math.markdown, *source, "{source}");
        assert_eq!(math.pending_start, None, "{source}");
    }
}

#[test]
fn ordinary_dollars_render_as_written() {
    for (source, expected) in [
        ("It costs $5 and $10.", "It costs $5 and $10."),
        (
            "Use $HOME, $HOME/$USER and $PATH:$HOME.",
            "Use $HOME, $HOME/$USER and $PATH:$HOME.",
        ),
        (
            "$('#id').hide(); $.ajax(url);",
            "$('#id').hide(); $.ajax(url);",
        ),
        ("Match ^foo$ and ^bar$.", "Match ^foo$ and ^bar$."),
        ("\\$x\\$ and \\$5", "$x$ and $5"),
        ("价格是$5和$10。", "价格是$5和$10。"),
        (
            "| Price |\n|---|\n| $5 and $10 |\n",
            " Price\n━━━━━━━━━━━━\n $5 and $10",
        ),
    ] {
        assert_eq!(plain(source, 80), expected, "{source}");
    }
}

/// Codex's rules take these as math, or not, though a reader might not:
/// pinned so a change to the rules is a decision.
#[test]
fn the_limits_of_codexs_rules() {
    for (source, expected) in [
        // A `$` closer followed by a non-alphanumeric: a formula.
        ("s/^a$/b$/", "s/^a/b/"),
        ("^foo$|^bar$", "^foo|ᵇar"),
        ("$0 ~ /x$/", "0 ~ /x/"),
        // A letter right after the closer, CJK too: not a formula.
        ("公式$x^2$很简单", "公式$x^2$很简单"),
        ("公式 $x^2$ 很简单", "公式 x² 很简单"),
        ("公式\\(x^2\\)很简单", "公式x²很简单"),
        // A `$$` after text pairs with the next `$$` not at a line's
        // start: a formula between them stays as written.
        (
            "echo $$ then $\\alpha$ then echo $$",
            "echo $$ then $\\alpha$ then echo $$",
        ),
        // A list marker or a quote's `>` is text before a `$$`: no display.
        ("- $$x^2$$", "• $$x^2$$"),
        ("> $$x^2$$", "> $$x^2$$"),
        // `\(…\)` and `\[…\]` have no guard: Markdown's escaped
        // parentheses and brackets become math, and an empty formula keeps
        // its backslashes.
        ("f\\(x\\) and foo\\(\\)", "fx and foo\\(\\)"),
        ("s/\\(a\\)\\(b\\)/\\2\\1/", "s/ab/\\2\\1/"),
        ("grep -E '\\(foo\\|bar\\)' file", "grep -E 'foo‖bar' file"),
        ("\\[note\\]", "note"),
        // A table's `\|` inside a formula is TeX's `\|`.
        ("| A |\n|---|\n| $a \\| b$ |\n", " A\n━━━━━━━\n a ‖ b"),
        // A `$$` or `\[` opening a line with no closer: the rest is its
        // source, Markdown included.
        (
            "$$ is the PID.\n\n**Bold** and `code`.",
            "$$ is the PID.\n\n**Bold** and `code`.",
        ),
    ] {
        assert_eq!(plain(source, 80), expected, "{source}");
    }
}

/// A formula is shown as text: never scanned for web addresses, never
/// linked, in prose, a table cell, a list or a quote; a bare URL beside it
/// still is.
#[test]
fn a_formula_is_never_linked() {
    for source in [
        "See $\\text{https://example.com/a}$ there.",
        "| A |\n|---|\n| $\\text{https://example.com/a}$ |\n",
        "- $\\text{https://example.com/a}$\n",
        "> $\\text{https://example.com/a}$\n",
        "$$\\text{https://example.com/a}$$",
        "Unsupported $\\unknown{https://example.com/a}$ there.",
    ] {
        for width in [None, Some(80), Some(20)] {
            let lines = render_markdown_lines(source, width, None, ListSpacing::AfterMultiline);
            let text: String = lines.iter().map(|line| line.line.to_string()).collect();
            assert!(text.contains("example.com"), "{source}: {text}");
            assert!(
                lines.iter().all(|line| line.hyperlinks.is_empty()),
                "{source} at {width:?}"
            );
        }
    }
    let lines = render_markdown_lines(
        "$\\alpha$ and https://example.com/b",
        Some(80),
        None,
        ListSpacing::AfterMultiline,
    );
    assert_eq!(lines[0].line.to_string(), "α and https://example.com/b");
    assert_eq!(lines[0].hyperlinks.len(), 1);
}
