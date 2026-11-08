// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Text;

use crate::markdown_render::render_markdown_text;
use insta::assert_snapshot;

use crate::markdown_render::render_markdown_text_with_width;

mod blockquotes;
mod lists;

#[test]
fn empty() {
    assert_eq!(render_markdown_text(""), Text::default());
}

#[test]
fn paragraph_single() {
    assert_eq!(
        render_markdown_text("Hello, world!"),
        Text::from("Hello, world!")
    );
}

#[test]
fn paragraph_soft_break() {
    assert_eq!(
        render_markdown_text("Hello\nWorld"),
        Text::from_iter(["Hello", "World"])
    );
}

#[test]
fn paragraph_multiple() {
    assert_eq!(
        render_markdown_text("Paragraph 1\n\nParagraph 2"),
        Text::from_iter(["Paragraph 1", "", "Paragraph 2"])
    );
}

#[test]
fn headings() {
    let md = "# Heading 1\n## Heading 2\n### Heading 3\n#### Heading 4\n##### Heading 5\n###### Heading 6\n";
    let text = render_markdown_text(md);
    let expected = Text::from_iter([
        Line::from_iter(["# ".bold().underlined(), "Heading 1".bold().underlined()]),
        Line::default(),
        Line::from_iter(["## ".bold(), "Heading 2".bold()]),
        Line::default(),
        Line::from_iter(["### ".bold().italic(), "Heading 3".bold().italic()]),
        Line::default(),
        Line::from_iter(["#### ".italic(), "Heading 4".italic()]),
        Line::default(),
        Line::from_iter(["##### ".italic(), "Heading 5".italic()]),
        Line::default(),
        Line::from_iter(["###### ".italic(), "Heading 6".italic()]),
    ]);
    assert_eq!(text, expected);
}

#[test]
fn inline_code() {
    let text = render_markdown_text("Example of `Inline code`");
    let expected = Line::from_iter(["Example of ".into(), "Inline code".cyan()]).into();
    assert_eq!(text, expected);
}

#[test]
fn strong() {
    assert_eq!(
        render_markdown_text("**Strong**"),
        Text::from(Line::from("Strong".bold()))
    );
}

#[test]
fn emphasis() {
    assert_eq!(
        render_markdown_text("*Emphasis*"),
        Text::from(Line::from("Emphasis".italic()))
    );
}

#[test]
fn strikethrough() {
    assert_eq!(
        render_markdown_text("~~Strikethrough~~"),
        Text::from(Line::from("Strikethrough".crossed_out()))
    );
}

#[test]
fn strong_emphasis() {
    let text = render_markdown_text("**Strong *emphasis***");
    let expected = Text::from(Line::from_iter([
        "Strong ".bold(),
        "emphasis".bold().italic(),
    ]));
    assert_eq!(text, expected);
}

#[test]
fn link() {
    let text = render_markdown_text("[Link](https://example.com)");
    let expected = Text::from(Line::from_iter([
        "Link".into(),
        " (".into(),
        "https://example.com".cyan().underlined(),
        ")".into(),
    ]));
    assert_eq!(text, expected);
}

#[test]
fn code_block_unhighlighted() {
    let text = render_markdown_text("```rust\nfn main() {}\n```\n");
    let expected = Text::from_iter([Line::from_iter(["", "fn main() {}"])]);
    assert_eq!(text, expected);
}

#[test]
fn code_block_multiple_lines_root() {
    let md = "```\nfirst\nsecond\n```\n";
    let text = render_markdown_text(md);
    let expected = Text::from_iter([
        Line::from_iter(["", "first"]),
        Line::from_iter(["", "second"]),
    ]);
    assert_eq!(text, expected);
}

#[test]
fn code_block_indented() {
    let md = "    function greet() {\n      console.log(\"Hi\");\n    }\n";
    let text = render_markdown_text(md);
    let expected = Text::from_iter([
        Line::from_iter(["    ", "function greet() {"]),
        Line::from_iter(["    ", "  console.log(\"Hi\");"]),
        Line::from_iter(["    ", "}"]),
    ]);
    assert_eq!(text, expected);
}

#[test]
fn horizontal_rule_renders_em_dashes() {
    let md = "Before\n\n---\n\nAfter\n";
    let text = render_markdown_text(md);
    let lines: Vec<String> = text
        .lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect();
    assert_eq!(lines, vec!["Before", "", "———", "", "After"]);
}

#[test]
fn code_block_with_inner_triple_backticks_outer_four() {
    let md = r#"````text
Here is a code block that shows another fenced block:

```md
# Inside fence
- bullet
- `inline code`
```
````
"#;
    let text = render_markdown_text(md);
    let lines: Vec<String> = text
        .lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect();
    assert_eq!(
        lines,
        vec![
            "Here is a code block that shows another fenced block:".to_string(),
            String::new(),
            "```md".to_string(),
            "# Inside fence".to_string(),
            "- bullet".to_string(),
            "- `inline code`".to_string(),
            "```".to_string(),
        ]
    );
}

#[test]
fn markdown_render_complex_snapshot() {
    let md = r#"# H1: Markdown Streaming Test
Intro paragraph with bold **text**, italic *text*, and inline code `x=1`.
Combined bold-italic ***both*** and escaped asterisks \*literal\*.
Auto-link: <https://example.com> and reference link [ref][r1].
Link with title: [hover me](https://example.com "Example") and mailto <mailto:test@example.com>.
Image: ![alt text](https://example.com/img.png "Title")
> Blockquote level 1
>> Blockquote level 2 with `inline code`
- Unordered list item 1
  - Nested bullet with italics _inner_
- Unordered list item 2 with ~~strikethrough~~
1. Ordered item one
2. Ordered item two with sublist:
   1) Alt-numbered subitem
- [ ] Task: unchecked
- [x] Task: checked with link [home](https://example.org)
---
Table below (alignment test):
| Left | Center | Right |
|:-----|:------:|------:|
| a    |   b    |     c |
Inline HTML: <sup>sup</sup> and <sub>sub</sub>.
HTML block:
<div style="border:1px solid #ccc;padding:2px">inline block</div>
Escapes: \_underscores\_, backslash \\, ticks ``code with `backtick` inside``.
Emoji shortcodes: :sparkles: :tada: (if supported).
Hard break test (line ends with two spaces)  
Next line should be close to previous.
Footnote reference here[^1] and another[^longnote].
Horizontal rule with asterisks:
***
Fenced code block (JSON):
```json
{ "a": 1, "b": [true, false] }
```
Fenced code with tildes and triple backticks inside:
~~~markdown
To close ``` you need tildes.
~~~
Indented code block:
    for i in range(3): print(i)
Definition-like list:
Term
: Definition with `code`.
Character entities: &amp; &lt; &gt; &quot; &#39;
[^1]: This is the first footnote.
[^longnote]: A longer footnote with a link to [Rust](https://www.rust-lang.org/).
Escaped pipe in text: a \| b \| c.
URL with parentheses: [link](https://example.com/path_(with)_parens).
[r1]: https://example.com/ref "Reference link title"
"#;

    let text = render_markdown_text(md);
    // Convert to plain text lines for snapshot (ignore styles)
    let rendered = text
        .lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_snapshot!(rendered);
}

#[test]
fn html_inline_is_verbatim() {
    let md = "Hello <span>world</span>!";
    let text = render_markdown_text(md);
    let expected: Text = Line::from_iter(["Hello ", "<span>", "world", "</span>", "!"]).into();
    assert_eq!(text, expected);
}

#[test]
fn html_block_is_verbatim_multiline() {
    let md = "<div>\n  <span>hi</span>\n</div>\n";
    let text = render_markdown_text(md);
    let expected = Text::from_iter([
        Line::from_iter(["<div>"]),
        Line::from_iter(["  <span>hi</span>"]),
        Line::from_iter(["</div>"]),
    ]);
    assert_eq!(text, expected);
}

#[test]
fn html_in_tight_ordered_item_soft_breaks_with_space() {
    let md = "1. Foo\n   <i>Bar</i>\n";
    let text = render_markdown_text(md);
    let expected = Text::from_iter([
        Line::from_iter(["1. ".light_blue(), "Foo".into()]),
        Line::from_iter(["   ", "<i>", "Bar", "</i>"]),
    ]);
    assert_eq!(text, expected);
}

#[test]
fn html_continuation_paragraph_in_unordered_item_indented() {
    let md = "- Item\n\n  <em>continued</em>\n";
    let text = render_markdown_text(md);
    let expected = Text::from_iter([
        Line::from_iter(["- ", "Item"]),
        Line::default(),
        Line::from_iter(["  ", "<em>", "continued", "</em>"]),
    ]);
    assert_eq!(text, expected);
}

fn lines_to_strings(text: &Text<'_>) -> Vec<String> {
    text.lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn wraps_plain_text_when_width_provided() {
    let markdown = "This is a simple sentence that should wrap.";
    let rendered = render_markdown_text_with_width(markdown, Some(16));
    let lines = lines_to_strings(&rendered);
    assert_eq!(
        lines,
        vec![
            "This is a simple".to_string(),
            "sentence that".to_string(),
            "should wrap.".to_string(),
        ]
    );
}

#[test]
fn wraps_list_items_preserving_indent() {
    let markdown = "- first second third fourth";
    let rendered = render_markdown_text_with_width(markdown, Some(14));
    let lines = lines_to_strings(&rendered);
    assert_eq!(
        lines,
        vec!["- first second".to_string(), "  third fourth".to_string(),]
    );
}

#[test]
fn wraps_nested_lists() {
    let markdown =
        "- outer item with several words to wrap\n  - inner item that also needs wrapping";
    let rendered = render_markdown_text_with_width(markdown, Some(20));
    let lines = lines_to_strings(&rendered);
    assert_eq!(
        lines,
        vec![
            "- outer item with".to_string(),
            "  several words to".to_string(),
            "  wrap".to_string(),
            "    - inner item".to_string(),
            "      that also".to_string(),
            "      needs wrapping".to_string(),
        ]
    );
}

#[test]
fn wraps_ordered_lists() {
    let markdown = "1. ordered item contains many words for wrapping";
    let rendered = render_markdown_text_with_width(markdown, Some(18));
    let lines = lines_to_strings(&rendered);
    assert_eq!(
        lines,
        vec![
            "1. ordered item".to_string(),
            "   contains many".to_string(),
            "   words for".to_string(),
            "   wrapping".to_string(),
        ]
    );
}

#[test]
fn wraps_blockquotes() {
    let markdown = "> block quote with content that should wrap nicely";
    let rendered = render_markdown_text_with_width(markdown, Some(22));
    let lines = lines_to_strings(&rendered);
    assert_eq!(
        lines,
        vec![
            "> block quote with".to_string(),
            "> content that should".to_string(),
            "> wrap nicely".to_string(),
        ]
    );
}

#[test]
fn wraps_blockquotes_inside_lists() {
    let markdown = "- list item\n  > block quote inside list that wraps";
    let rendered = render_markdown_text_with_width(markdown, Some(24));
    let lines = lines_to_strings(&rendered);
    assert_eq!(
        lines,
        vec![
            "- list item".to_string(),
            "  > block quote inside".to_string(),
            "  > list that wraps".to_string(),
        ]
    );
}

#[test]
fn wraps_list_items_containing_blockquotes() {
    let markdown = "1. item with quote\n   > quoted text that should wrap";
    let rendered = render_markdown_text_with_width(markdown, Some(24));
    let lines = lines_to_strings(&rendered);
    assert_eq!(
        lines,
        vec![
            "1. item with quote".to_string(),
            "   > quoted text that".to_string(),
            "   > should wrap".to_string(),
        ]
    );
}

#[test]
fn does_not_wrap_code_blocks() {
    let markdown = "````\nfn main() { println!(\"hi from a long line\"); }\n````";
    let rendered = render_markdown_text_with_width(markdown, Some(10));
    let lines = lines_to_strings(&rendered);
    assert_eq!(
        lines,
        vec!["fn main() { println!(\"hi from a long line\"); }".to_string(),]
    );
}
