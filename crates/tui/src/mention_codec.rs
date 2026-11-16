//! Bound composer mentions use stable links in queued messages and persisted text.

use std::ops::Range;

pub(crate) struct Mention {
    pub(crate) range: Range<usize>,
    pub(crate) path: String,
}

pub(crate) fn encode(label: &str, path: &str) -> String {
    let label = label
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!("[{label}]({path})")
}

pub(crate) fn decode(text: &str) -> (String, Vec<Mention>) {
    let mut output = String::with_capacity(text.len());
    let mut mentions = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find('[') {
        let start = cursor + relative;
        output.push_str(&text[cursor..start]);
        if let Some((label, path, end)) = linked_mention(&text[start..]) {
            let start_range = output.len();
            output.push_str(&label);
            mentions.push(Mention {
                range: start_range..output.len(),
                path: path.to_owned(),
            });
            cursor = start + end;
        } else {
            output.push('[');
            cursor = start + 1;
        }
    }
    output.push_str(&text[cursor..]);
    (output, mentions)
}

fn linked_mention(text: &str) -> Option<(String, &str, usize)> {
    if !text[1..].starts_with(['@', '$']) {
        return None;
    }
    let mut escaped = false;
    let mut label = String::new();
    for (index, ch) in text[1..].char_indices() {
        if escaped {
            label.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ']' {
            let path_start = index + 3;
            if !text[index + 1..].starts_with("](") {
                return None;
            }
            let path_end = path_start + text[path_start..].find(')')?;
            let path = &text[path_start..path_end];
            return ["skill://", "plugin://", "mcp://", "thread://"]
                .iter()
                .any(|prefix| path.starts_with(prefix))
                .then_some((label, path, path_end + 1));
        } else {
            label.push(ch);
        }
    }
    None
}

pub(crate) fn display_text(text: &str) -> String {
    decode(text).0
}

#[cfg(test)]
mod tests;
