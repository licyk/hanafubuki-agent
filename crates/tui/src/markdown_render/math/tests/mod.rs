// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Codex's inline math tests: the Unicode notation, Markdown left to
//! Markdown around a formula, and what is not math.

mod display;
mod ordinary_text;

use super::render::{render, MathMode};
use crate::markdown_render::render_markdown_text_with_width;

fn plain(source: &str, width: usize) -> String {
    render_markdown_text_with_width(source, Some(width))
        .lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn unicode_math_inline_snapshot() {
    insta::assert_snapshot!(plain(
        r"Inline $\alpha^2 + \beta_{10}$ on $\mathbb{R}^n$.
Root $\sqrt{x^2+y^2}$ and fraction $a/\frac{b}{c}$.
Percent: $3.8\%$ and $100000 \times 3.8\% \times \frac{360}{365} \approx 3747.95$.
Unsupported: $\unknown{x_y}$ and $\frac{a}{b}^2$.
Angles: <$x$>, <\(\alpha\)>, and <$\unknown{x}$>.
Code: `$\alpha$`; money: $5 and $10; shell: $HOME.",
        80
    ));
}

#[test]
fn unicode_math_inline_narrow_snapshot() {
    insta::assert_snapshot!(plain(
        r"Words $\alpha^2 + \beta_1$ more words and $\sqrt{x}$.",
        16
    ));
}

#[test]
fn unicode_math_accents_and_symbols_snapshot() {
    insta::assert_snapshot!(plain(
        r"Operators: $\hat H^2 + \hbar^2$, $\ell$, $A\dagger$, $B\ddagger$.
Accents: $\bar{x}_1$, $\tilde{x}^2$, $\vec{v}_i$, $\dot{x}$, $\ddot{x}$.
Greek accents: $\hat{\Psi}^2$, $\bar{\varrho}$, $\tilde{\varsigma}$.
Other symbols: $\varkappa+\varpi$, $\Re z+\Im z$, $\aleph_0$, $\hat{\imath}$.
Relations: $a\propto b\sim c\simeq d\ll e\gg f$.
Geometry: $a\perp b\parallel c$, $\angle ABC\cong\angle DEF$.
Bounds: $a\lesssim b\gtrsim c$; operators: $a\oplus b\otimes c\odot d$.
Sets: $A\supseteq B\supset C\ni x$, $A\setminus B=\varnothing$.
Logic: $\neg P\land Q\lor R\implies S\iff T\impliedby U$, $\nexists x$.
Arrows: $a\leftrightarrow b\mapsto c\Leftarrow d$, $\uparrow\downarrow\updownarrow$.
Integrals: $\iint f$, $\iiint g$, $\oint h$.
Collections: $\coprod A$, $\bigcup B$, $\bigcap C$.
Punctuation: $x\prime$, $a\circ b\bullet c$, $\vdots\ddots$.",
        80
    ));
}

#[test]
fn unicode_math_named_delimiters_snapshot() {
    insta::assert_snapshot!(plain(
        r"Inner product: $\left\langle\hat H\right\rangle$.
Rounding: $\lfloor x\rfloor + \left\lceil y\right\rceil$.
Norms: $\left\lVert v\right\rVert$, $\lvert x\rvert$, $\left\|w\right\|$.
Sets: $\left\{x\right\}$, $\lbrace y\rbrace$, $\lbrack z\rbrack$.
Angles: $\left<x\right>$.",
        80
    ));
}

#[test]
fn unicode_math_accents_reject_ambiguous_arguments() {
    for source in [
        r"\hat{xy}",
        r"\bar{x+y}",
        r"\tilde{x^2}",
        r"\vec{\frac{x}{y}}",
        r"\dot{}",
        r"\ddot{ }",
        r"\hat",
        "\\hat{\u{0302}}",
    ] {
        for mode in [MathMode::Inline, MathMode::Display] {
            assert_eq!(render(source, mode), None, "{source}");
        }
        assert_eq!(
            plain(&format!("\\({source}\\)"), 80),
            format!("\\({source}\\)")
        );
    }
}

#[test]
fn unicode_math_preserves_markdown_contexts() {
    for (source, expected) in [
        (
            r"**$\alpha_1$** &amp; \$5.00 and $\beta^2$",
            "α₁ & $5.00 and β²",
        ),
        (
            r"`$\alpha$` $HOME ${HOME} $(echo x) $5 and $10",
            r"$\alpha$ $HOME ${HOME} $(echo x) $5 and $10",
        ),
        (
            r"[x](https://example.com/$HOME) and $\alpha$",
            "x (https://example.com/$HOME) and α",
        ),
        (r"$\left. x \right|$ $\text{x_y^z}$", " x | x_y^z"),
        (r"\(\alpha^2\)", "α²"),
        (r"Value: <$x$> and <\(\alpha\)>.", "Value: <x> and <α>."),
        (r"$USD$+$\alpha$", "$USD$+α"),
        (r"Costs $5; $\alpha$ and $x^2$.", "Costs $5; α and x²."),
        (r"$HOME then $\alpha$", "$HOME then α"),
        (
            r"Run echo $$ to print PID. Then $\alpha$",
            "Run echo $$ to print PID. Then α",
        ),
    ] {
        assert_eq!(plain(source, 80), expected);
    }
}

#[test]
fn unicode_math_bounds_and_unsupported_input() {
    for source in [
        r"\unknown{x}",
        r"\frac{a}",
        "{x",
        "x}",
        "^2",
        "x^2^3",
        "{a+b}^2",
        "{x^2}^3",
        r"\frac{a}{b}^2",
        r"\sqrt[3]{x}",
        r"\sqrt [3]{x}",
        r"\left x",
        r"\left\alpha x\right\rangle",
        r"\left\ ",
        r"\text{\alpha}",
        r"\begin{matrix}a&b\end{matrix}",
    ] {
        assert_eq!(render(source, MathMode::Inline), None, "{source}");
        assert_eq!(
            plain(&format!("\\({source}\\)"), 80),
            format!("\\({source}\\)")
        );
    }
    for source in [
        format!("{}x{}", "{".repeat(40), "}".repeat(40)),
        "x".repeat(300),
        format!("{}x", "\\sqrt".repeat(100)),
    ] {
        assert_eq!(render(&source, MathMode::Inline), None);
    }
}

#[test]
fn unicode_math_oversized_display_does_not_hide_following_inline_math() {
    for (open, close) in [("$$", "$$"), (r"\[", r"\]")] {
        let source = format!(
            "{open}\n{}\n{close}\n\nAfter $\\alpha$.\n",
            "x".repeat(5000)
        );
        assert!(plain(&source, 80).ends_with("After α."));
    }
}

/// Inline math in a table cell, a list item and a quote, beside CJK text;
/// a display in a list item on one row.
#[test]
fn math_in_tables_lists_and_quotes() {
    assert_eq!(
        plain(
            "| 名前 | Value |\n|---|---|\n| 根 | $\\sqrt{x}$ and $\\alpha_1$ |\n",
            80
        ),
        " 名前    Value\n━━━━━━  ━━━━━━━━━━━━━\n 根      √(x) and α₁"
    );
    assert_eq!(
        plain(
            "- 公式 $x^2$ 很简单\n- $\\alpha$\n\n  $$\\int_0^1 x\\,dx$$\n\n> 引用 \\(\\beta^2\\)。\n",
            80
        ),
        "• 公式 x² 很简单\n• α\n\n  ∫₀¹ x dx\n\n> 引用 β²。"
    );
}

/// Inline and display math at every width: nothing panics, and from 12
/// columns on no row is wider than the width (a display that does not fit
/// is its source, wrapped).
#[test]
fn math_at_every_width() {
    let source = "Inline $\\alpha^2 + \\beta_{10}$ with 中文 $x_1$ 和 \\(\\sqrt{y}\\)。\n\n$$\nx=\\frac{-b\\pm\\sqrt{b^2-4ac}}{2a}\n$$\n\n$$A=\\begin{pmatrix}1&2\\\\3&4\\end{pmatrix}$$\n\n- Item $\\frac{a}{b}$\n\n| A | B |\n|---|---|\n| $x^2$ | $\\gamma$ |\n";
    for width in 0..=120 {
        let rendered = render_markdown_text_with_width(source, Some(width));
        if width >= 12 {
            for line in &rendered.lines {
                assert!(
                    crate::wrapping::display_width(&line.to_string()) <= width,
                    "{width}: {line}"
                );
            }
        }
    }
}
