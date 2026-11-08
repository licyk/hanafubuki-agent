use super::{MAX_HTML_DEPTH, MAX_HTML_NODES};
use crate::model::MAX_WEB_SEARCH_URL_BYTES;
use dom_query::NodeRef;
use hanafubuki_agent_foundation::url_policy::UrlPolicy;
use reqwest::Url;

pub(super) struct MarkdownRenderer<'a> {
    base_url: &'a Url,
    maximum_bytes: usize,
    pub(super) output: String,
    nodes: usize,
    pub(super) truncated: bool,
}

impl<'a> MarkdownRenderer<'a> {
    pub(super) fn new(base_url: &'a Url, maximum_bytes: usize) -> Self {
        Self {
            base_url,
            maximum_bytes,
            output: String::new(),
            nodes: 0,
            truncated: false,
        }
    }

    pub(super) fn render(&mut self, node: NodeRef<'_>, depth: usize) {
        if self.truncated {
            return;
        }
        self.nodes += 1;
        if self.nodes > MAX_HTML_NODES || depth > MAX_HTML_DEPTH {
            self.truncated = true;
            return;
        }
        if node.is_text() {
            self.push_words(node.immediate_text().as_ref());
            return;
        }
        let name = node
            .node_name()
            .map(|name| name.to_string())
            .unwrap_or_default();
        match name.as_str() {
            "script" | "style" | "noscript" | "template" | "svg" | "canvas" | "img" => {}
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.block_break();
                let level = name[1..].parse::<usize>().unwrap_or(2);
                self.push(&"#".repeat(level));
                self.push(" ");
                self.push_words(node.text().as_ref());
                self.block_break();
            }
            "a" => {
                let label = normalize_words(node.text().as_ref());
                if label.is_empty() {
                    return;
                }
                let link = node
                    .attr("href")
                    .and_then(|href| safe_output_link(self.base_url, href.as_ref()));
                if let Some(link) = link {
                    self.push("[");
                    self.push(&escape_markdown_label(&label));
                    self.push("](");
                    self.push(&link);
                    self.push(")");
                } else {
                    self.push_words(&label);
                }
            }
            "pre" => {
                self.block_break();
                self.push("```\n");
                self.push(node.text().as_ref());
                self.push("\n```\n\n");
            }
            "code" => {
                self.push("`");
                self.push(node.text().as_ref());
                self.push("`");
            }
            "li" => {
                self.line_break();
                self.push("- ");
                for child in node.children_it(false) {
                    self.render(child, depth + 1);
                }
                self.line_break();
            }
            "blockquote" => {
                self.block_break();
                self.push("> ");
                for child in node.children_it(false) {
                    self.render(child, depth + 1);
                }
                self.block_break();
            }
            "br" => self.line_break(),
            "p" | "div" | "section" | "article" | "main" | "header" | "table" | "tr" => {
                self.block_break();
                for child in node.children_it(false) {
                    self.render(child, depth + 1);
                }
                self.block_break();
            }
            _ => {
                for child in node.children_it(false) {
                    self.render(child, depth + 1);
                }
            }
        }
    }

    fn push_words(&mut self, value: &str) {
        let words = normalize_words(value);
        if words.is_empty() {
            return;
        }
        if !self.output.is_empty()
            && !self.output.ends_with([' ', '\n', '`', '['])
            && !words.starts_with([',', '.', ';', ':', '!', '?', ')', ']'])
        {
            self.push(" ");
        }
        self.push(&words);
    }

    fn push(&mut self, value: &str) {
        if self.truncated || value.is_empty() {
            return;
        }
        let remaining = self.maximum_bytes.saturating_sub(self.output.len());
        if value.len() <= remaining {
            self.output.push_str(value);
            return;
        }
        let (prefix, _) = truncate_utf8(value, remaining);
        self.output.push_str(&prefix);
        self.truncated = true;
    }

    fn line_break(&mut self) {
        if !self.output.ends_with('\n') {
            self.push("\n");
        }
    }

    fn block_break(&mut self) {
        if self.output.is_empty() || self.output.ends_with("\n\n") {
            return;
        }
        if self.output.ends_with('\n') {
            self.push("\n");
        } else {
            self.push("\n\n");
        }
    }
}

fn normalize_words(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn normalize_blank_lines(value: &str) -> String {
    let mut output = String::new();
    let mut blank = false;
    for line in value.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            if !blank && !output.is_empty() {
                output.push('\n');
            }
            blank = true;
        } else {
            output.push_str(line);
            output.push('\n');
            blank = false;
        }
    }
    output.trim().to_string()
}

fn escape_markdown_label(value: &str) -> String {
    value.replace('[', "\\[").replace(']', "\\]")
}

fn safe_output_link(base: &Url, raw: &str) -> Option<String> {
    if raw.len() > MAX_WEB_SEARCH_URL_BYTES {
        return None;
    }
    let mut url = base.join(raw).ok()?;
    UrlPolicy::PAGE.check(&url).ok()?;
    url.set_fragment(None);
    let rendered = url.to_string();
    (rendered.len() <= MAX_WEB_SEARCH_URL_BYTES).then_some(rendered)
}

pub(super) fn truncate_utf8(value: &str, maximum_bytes: usize) -> (String, bool) {
    if value.len() <= maximum_bytes {
        return (value.to_string(), false);
    }
    let mut boundary = maximum_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (value[..boundary].to_string(), true)
}
