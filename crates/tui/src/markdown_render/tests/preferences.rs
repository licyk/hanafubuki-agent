// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `tui.rendering`: a construct turned off is shown as the model wrote it.

use hanafubuki_agent::AgentTuiRendering;

use crate::markdown::append_markdown_agent;
use crate::markdown_render::preferences;
use crate::markdown_render::render_markdown_text;

fn agent_text(source: &str) -> String {
    let mut lines = Vec::new();
    append_markdown_agent(source, None, None, &mut lines);
    lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn disabled_renderers_preserve_source_independently() {
    let samples = [
        "```mermaid\nflowchart LR\nA[Request] --> B[Reply]\n```\n",
        "Inline $\\alpha_1$ and \\(x^{2}\\).\n\n\\[\n\\frac{a}{b}\n\\]\n",
        "| **Name** | Value |\n|---|---|\n| `a` | path\\_1 &amp; \\* |\n",
        "- [ ] Pending\n- [x] Done\n- Ordinary\n",
    ];
    let defaults = samples.map(render_markdown_text);
    let mut snapshot = Vec::new();
    for rendering in [
        AgentTuiRendering {
            mermaid: false,
            ..Default::default()
        },
        AgentTuiRendering {
            math: false,
            ..Default::default()
        },
        AgentTuiRendering {
            tables: false,
            ..Default::default()
        },
        AgentTuiRendering {
            lists: false,
            ..Default::default()
        },
    ] {
        preferences::set(rendering);
        for (index, source) in samples.iter().enumerate() {
            let rendered = render_markdown_text(source);
            if [
                rendering.mermaid,
                rendering.math,
                rendering.tables,
                rendering.lists,
            ][index]
            {
                assert_eq!(rendered, defaults[index]);
            } else {
                snapshot.push(format!("{rendering:?}\n{rendered}"));
            }
        }
    }
    insta::assert_snapshot!(snapshot.join("\n\n---\n\n"));
}

#[test]
fn disabled_tables_keep_markdown_fences_and_cell_markup() {
    preferences::set(AgentTuiRendering {
        tables: false,
        ..Default::default()
    });
    for fence in [
        "```md",
        "```markdown",
        "~~~markdown",
        "```MARKDOWN title=example",
    ] {
        let closer = &fence[..3];
        let source = format!("{fence}\n| A | B |\n|---|---|\n| **a** | b |\n{closer}\n");
        assert_eq!(agent_text(&source), source.trim_end());
        assert_eq!(render_markdown_text(&source).to_string(), source.trim_end());
    }
    let quoted =
        "> ```md\n> Intro\n>\n>| A | B |\n> |---|---|\n> | a | b |\n>\n>> literal\n> ```\n";
    insta::assert_snapshot!(
        "disabled_tables_keep_mixed_quote_spacing",
        render_markdown_text(quoted).to_string()
    );
    // Four-space continuation indentation must not hide the table inside a
    // list's fence.
    for (marker, indent) in [("1.  ", "    "), ("> > 1.  ", ">>     ")] {
        let listed = format!(
            "{marker}```md\n{indent}| A | B |\n{indent}|---|---|\n{indent}| a | b |\n{indent}```\n"
        );
        let text = render_markdown_text(&listed).to_string();
        assert!(text.contains("```md"), "{text}");
        assert_eq!(text.matches("```").count(), 2, "{text}");
        assert!(text.contains("| a | b |"), "{text}");
    }
    let prose = "```markdown\n**Not a table**\n```\n";
    assert_eq!(render_markdown_text(prose).to_string(), "**Not a table**");
}

#[test]
fn disabled_lists_keep_dashes_and_brackets() {
    preferences::set(AgentTuiRendering {
        lists: false,
        ..Default::default()
    });
    assert_eq!(
        render_markdown_text("- one\n  - [x] two\n1. [ ] three\n").to_string(),
        "- one\n    - [x] two\n\n1. [ ] three"
    );
}

#[test]
fn disabled_mermaid_keeps_unsupported_source_without_notice() {
    preferences::set(AgentTuiRendering {
        mermaid: false,
        ..Default::default()
    });
    let source = "```mermaid\npie\n\"Cats\": 2\n```\n";
    let rendered = render_markdown_text(source);
    preferences::set(AgentTuiRendering::default());
    assert_eq!(
        rendered.to_string(),
        render_markdown_text(&source.replacen("mermaid", "unknown", 1)).to_string(),
    );
}

/// Math off shows each formula as its source, its TeX as written: the
/// masker still finds it, so neither its escapes nor its `*`/`_` are read
/// as Markdown (Codex's behaviour, which differs there from the renderer
/// before math: `\(` was an escaped `(`).
#[test]
fn disabled_math_shows_each_formula_as_written() {
    preferences::set(AgentTuiRendering {
        math: false,
        ..Default::default()
    });
    for source in [
        r"Inline \(x^{2}\) and \(y_1\) in prose.",
        r"Products $a*b*c$ and $x_1 + y_1$.",
        "$$\n\\frac{a}{b}\n$$",
        "Open\n\n$$\n**not bold**\n",
    ] {
        assert_eq!(render_markdown_text(source).to_string(), source.trim_end());
    }
    // Ordinary text is unchanged either way.
    assert_eq!(
        render_markdown_text("It costs $5 and *$10*.").to_string(),
        "It costs $5 and $10."
    );
}
