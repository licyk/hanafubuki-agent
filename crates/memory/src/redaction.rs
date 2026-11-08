const MAX_REDACTION_INPUT_BYTES: usize = 512 * 1024;

/// Bounded, best-effort secret redaction for memory model input and output.
/// Exact provider credentials should be supplied by the application boundary;
/// this function additionally catches common inline key/value forms.
pub fn redact_memory_text(text: &str, exact_secrets: &[String]) -> String {
    let bounded = if text.len() > MAX_REDACTION_INPUT_BYTES {
        &text[..floor_char_boundary(text, MAX_REDACTION_INPUT_BYTES)]
    } else {
        text
    };
    let mut output = bounded.to_string();
    for secret in exact_secrets {
        if secret.len() >= 8 {
            output = output.replace(secret, "[REDACTED]");
        }
    }
    output
        .lines()
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let sensitive = [
        "authorization:",
        "api_key=",
        "api-key=",
        "apikey=",
        "token=",
        "password=",
        "secret=",
        "private_key=",
        "\"authorization\":",
        "\"api_key\":",
        "\"apikey\":",
        "\"token\":",
        "\"password\":",
        "\"secret\":",
        "\"private_key\":",
    ];
    if let Some(index) = sensitive
        .iter()
        .filter_map(|needle| lower.find(needle))
        .min()
    {
        let delimiter = line[index..]
            .find([':', '='])
            .map(|relative| index + relative + 1)
            .unwrap_or(index);
        return format!("{} [REDACTED]", line[..delimiter].trim_end());
    }
    line.to_string()
}

pub(crate) fn floor_char_boundary(value: &str, maximum: usize) -> usize {
    let mut boundary = maximum.min(value.len());
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}
