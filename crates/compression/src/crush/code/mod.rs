// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the code-trim strategy of `kernel/src/crush.ts` of acp-kernel 0.0.189 at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: lines are scanned as bytes (every character
// the scanners look for is ASCII), and the JavaScript scanner lives in its
// own module.

mod javascript;
mod python;

use std::sync::LazyLock;

use regex::Regex;

use super::log::split_lines;
use crate::text::utf16_len;

/// A trimmed file must be shorter than this share of the original.
const CODE_KEEP_RATIO: f64 = 0.97;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Language {
    Python,
    JavaScript,
}

/// Trims source code: full-line comments, docstrings and blank-line runs
/// become one `[acp-crush: elided N lines]` marker; code lines, strings and
/// trailing comments stay byte for byte. Python and JavaScript/TypeScript
/// (sound for other `//`-comment languages); `None` when the language is
/// not detected with confidence, a scanner gives up, or too little is saved.
pub fn crush_code(text: &str) -> Option<String> {
    match detect_language(text)? {
        Language::Python => python::trim(text),
        Language::JavaScript => javascript::trim(text),
    }
}

struct Patterns {
    shebang: Regex,
    py_def: Regex,
    py_class: Regex,
    js_from: Regex,
    js_import: Regex,
    py_import: Regex,
    js_declaration: Regex,
    js_keyword: Regex,
    statement_end: Regex,
    py_self: Regex,
    py_keyword: Regex,
    js_runtime: Regex,
}

// JavaScript's `\w`, `\d` and `\b` are ASCII; the patterns say so.
static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(|| {
    let regex = |pattern: &str| Regex::new(pattern).ok();
    Some(Patterns {
        shebang: regex(r"^\s*#!.*python")?,
        py_def: regex(r"^\s*(async\s+)?def\s+[A-Za-z0-9_]")?,
        py_class: regex(r"^\s*class\s+[A-Za-z0-9_]")?,
        js_from: regex(r#"(?-u:\b)from\s+['"]"#)?,
        js_import: regex(r#"^\s*import\s+['"]"#)?,
        py_import: regex(r"^\s*(import|from)\s+[A-Za-z_]")?,
        js_declaration: regex(r"^\s*(function(?-u:\b)|const\s|let\s|var\s)")?,
        js_keyword: regex(r"^\s*(public|private|protected|static|final|void|return)(?-u:\b)")?,
        statement_end: regex(r";\s*$")?,
        py_self: regex(r"(?-u:\b)self\.")?,
        py_keyword: regex(r"^\s*(elif|except|yield)(?-u:\b)")?,
        js_runtime: regex(
            r"(?-u:\b)console\.log(?-u:\b)|(?-u:\b)require\s*\(|module\.exports|(?-u:\b)await\s",
        )?,
    })
});

/// Python or JavaScript by a score over the first 300 lines, when one
/// clearly outweighs the other.
pub(super) fn detect_language(text: &str) -> Option<Language> {
    let patterns = PATTERNS.as_ref()?;
    let mut python = 0u32;
    let mut javascript = 0u32;
    for line in split_lines(text).into_iter().take(300) {
        if patterns.shebang.is_match(line) {
            python += 4;
        }
        if patterns.py_def.is_match(line) || patterns.py_class.is_match(line) {
            python += 2;
        }
        if patterns.js_from.is_match(line) || patterns.js_import.is_match(line) {
            javascript += 2;
        } else if patterns.py_import.is_match(line) {
            python += 1;
        }
        if patterns.js_declaration.is_match(line) {
            javascript += 2;
        }
        if line.contains("=>") {
            javascript += 1;
        }
        if patterns.js_keyword.is_match(line) {
            javascript += 1;
        }
        if patterns.statement_end.is_match(line) && !line.trim().is_empty() {
            javascript += 1;
        }
        if patterns.py_self.is_match(line) || patterns.py_keyword.is_match(line) {
            python += 1;
        }
        if patterns.js_runtime.is_match(line) {
            javascript += 1;
        }
    }
    let (python, javascript) = (f64::from(python), f64::from(javascript));
    if python >= 3.0 && python > javascript * 1.5 {
        Some(Language::Python)
    } else if javascript >= 4.0 && javascript > python * 1.5 {
        Some(Language::JavaScript)
    } else {
        None
    }
}

/// Collects the kept lines and the elided ones as markers.
struct Emitter {
    lines: Vec<String>,
    pending: usize,
    comment: &'static str,
}

impl Emitter {
    fn new(comment: &'static str) -> Self {
        Self {
            lines: Vec::new(),
            pending: 0,
            comment,
        }
    }

    fn elide(&mut self) {
        self.pending += 1;
    }

    fn flush(&mut self) {
        if self.pending > 0 {
            let plural = if self.pending == 1 { "" } else { "s" };
            self.lines.push(format!(
                "{} [acp-crush: elided {} line{plural}]",
                self.comment, self.pending
            ));
            self.pending = 0;
        }
    }

    fn keep(&mut self, line: &str) {
        self.lines.push(line.to_owned());
    }

    /// The trimmed text, if it has fewer lines and saves enough.
    fn finish(mut self, original: &str) -> Option<String> {
        self.flush();
        if self.lines.len() >= split_lines(original).len() {
            return None;
        }
        let newline = if original.ends_with('\n') { "\n" } else { "" };
        let out = format!("{}{newline}", self.lines.join("\n"));
        ((utf16_len(&out) as f64) < utf16_len(original) as f64 * CODE_KEEP_RATIO).then_some(out)
    }
}
