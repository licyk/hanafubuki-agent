//! `SKILL.md` frontmatter.
//!
//! The contract is deliberately the same one Claude and Codex accept: a YAML
//! block delimited by `---`, carrying a `description` and optionally a `name`.
//! A Skill that omits `name` is named after its folder. Unknown keys are
//! ignored rather than rejected — third-party packages carry `argument-hint`,
//! `model`, `author`, and whatever else their own tooling reads, and none of it
//! is this catalog's business. A known key is read on its own too: a
//! `metadata.version: 1.0` or a numeric `license` is dropped with an issue
//! beside the catalog, and the Skill loads without it.
//!
//! Third-party frontmatter is also frequently not quite YAML: `description:
//! Build for AWS: ECS` and `argument-hint: <duration: e.g. 7d>` both parse as
//! mappings and fail. The line-oriented repair below quotes exactly those
//! scalars and re-parses, so prose written by hand loads while genuinely
//! malformed YAML still surfaces as an error.

use serde_yaml::{Mapping, Value};

use crate::model::SkillError;
use crate::yaml_fields::{field, kind, optional_mapping, optional_string};

const MAX_NAME_CHARS: usize = 96;
const MAX_DESCRIPTION_BYTES: usize = 4096;
const MAX_LICENSE_BYTES: usize = 256;
const MAX_VERSION_BYTES: usize = 128;

/// Validated frontmatter, plus the document it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedSkill {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) short_description: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) version: Option<String>,
    /// The normalized complete document, frontmatter included.
    pub(crate) document: String,
    /// What was ignored or shortened to load the Skill, for the catalog's
    /// issues. A Skill with issues still loads.
    pub(crate) issues: Vec<String>,
}

/// Parse a `SKILL.md`. `default_name` is the Skill's folder name, used when the
/// frontmatter does not name it.
///
/// `name` and `description` are what make a Skill selectable, so a value of
/// the wrong type there still fails it. Every other known key is read on its
/// own: a wrong type drops that key with an issue. A name or description past
/// its bound is cut on a character boundary, also with an issue.
pub(crate) fn parse_skill(bytes: &[u8], default_name: &str) -> Result<ParsedSkill, SkillError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| SkillError::InvalidManifest("SKILL.md must be UTF-8".to_string()))?
        .replace("\r\n", "\n");
    let frontmatter = extract_frontmatter(&text).ok_or_else(|| {
        SkillError::InvalidManifest(
            "SKILL.md must open with a YAML frontmatter block delimited by ---".to_string(),
        )
    })?;
    let parsed = match serde_yaml::from_str::<Value>(&frontmatter) {
        Ok(parsed) => Ok(parsed),
        Err(original) => match repair_frontmatter_scalar_fields(&frontmatter) {
            Some(repaired) => serde_yaml::from_str(&repaired).map_err(|_| original),
            None => Err(original),
        },
    }
    .map_err(|error| {
        SkillError::InvalidManifest(format!("frontmatter is not valid YAML: {error}"))
    })?;
    let Value::Mapping(fields) = parsed else {
        return Err(SkillError::InvalidManifest(format!(
            "frontmatter must be a mapping of keys, found {}",
            kind(&parsed)
        )));
    };
    let mut issues = Vec::new();

    let mut name = essential_string(&fields, "name")?
        .as_deref()
        .map(single_line)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| single_line(default_name));
    if name.is_empty() {
        return Err(SkillError::InvalidManifest(format!(
            "name must be 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        name = name
            .chars()
            .take(MAX_NAME_CHARS)
            .collect::<String>()
            .trim_end()
            .to_string();
        issues.push(format!(
            "name is longer than {MAX_NAME_CHARS} characters; shortened"
        ));
    }
    let mut description = essential_string(&fields, "description")?
        .as_deref()
        .map(single_line)
        .unwrap_or_default();
    if description.is_empty() {
        return Err(SkillError::InvalidManifest(
            "frontmatter must carry a description".to_string(),
        ));
    }
    if description.len() > MAX_DESCRIPTION_BYTES {
        truncate_on_char_boundary(&mut description, MAX_DESCRIPTION_BYTES);
        issues.push(format!(
            "description is longer than {MAX_DESCRIPTION_BYTES} bytes; shortened"
        ));
    }

    let license = optional_string(&fields, &["license"], "license", &mut issues);
    let metadata = optional_mapping(&fields, "metadata", "metadata", &mut issues);
    let (version, short_description) = match metadata {
        Some(metadata) => (
            optional_string(metadata, &["version"], "metadata.version", &mut issues),
            optional_string(
                metadata,
                &["short-description", "short_description"],
                "metadata.short-description",
                &mut issues,
            ),
        ),
        None => (None, None),
    };

    Ok(ParsedSkill {
        name,
        description,
        short_description: short_description
            .as_deref()
            .map(single_line)
            .filter(|value| !value.is_empty()),
        license: bounded_optional(license.as_deref(), MAX_LICENSE_BYTES),
        version: bounded_optional(version.as_deref(), MAX_VERSION_BYTES),
        document: text.trim().to_string(),
        issues,
    })
}

/// `name` or `description`: absent is the caller's to decide, a string is the
/// value, and anything else invalidates the Skill.
fn essential_string(fields: &Mapping, key: &str) -> Result<Option<String>, SkillError> {
    match field(fields, &[key]) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(other) => Err(SkillError::InvalidManifest(format!(
            "{key} must be a string, found {}",
            kind(other)
        ))),
    }
}

/// Cut `value` to at most `maximum` bytes without splitting a character.
fn truncate_on_char_boundary(value: &mut String, maximum: usize) {
    let mut end = maximum.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    let trimmed = value.trim_end().len();
    value.truncate(trimmed);
}

/// A folder name for a Skill the user just named. Lowercase, hyphen-joined, and
/// stripped of anything a filesystem or an id would object to.
pub fn slug_from_name(name: &str) -> String {
    let mut slug = String::new();
    let mut pending_hyphen = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_hyphen && !slug.is_empty() {
                slug.push('-');
            }
            pending_hyphen = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            pending_hyphen = true;
        }
    }
    slug.truncate(64);
    slug.trim_matches('-').to_string()
}

fn bounded_optional(value: Option<&str>, maximum: usize) -> Option<String> {
    value
        .map(single_line)
        .filter(|value| !value.is_empty() && value.len() <= maximum)
}

fn single_line(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_frontmatter(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    // A byte-order mark survives a UTF-8 decode and would otherwise make the
    // opening delimiter unrecognisable.
    if !matches!(lines.next(), Some(line) if line.trim_start_matches('\u{feff}').trim() == "---") {
        return None;
    }
    let mut frontmatter = Vec::new();
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        frontmatter.push(line);
    }
    (closed && !frontmatter.is_empty()).then(|| frontmatter.join("\n"))
}

/// Quote scalars that read as prose but parse as YAML structure. Line-oriented
/// on purpose: anything it does not recognise is left exactly as written, so a
/// genuinely broken document still fails.
fn repair_frontmatter_scalar_fields(frontmatter: &str) -> Option<String> {
    let mut changed = false;
    let mut block_scalar_indent: Option<usize> = None;
    let mut repaired = Vec::new();
    for line in frontmatter.lines() {
        let indent = line
            .chars()
            .take_while(|character| *character == ' ')
            .count();
        if let Some(block_indent) = block_scalar_indent {
            if line.trim().is_empty() || indent > block_indent {
                repaired.push(line.to_string());
                continue;
            }
            block_scalar_indent = None;
        }
        let Some((key, value)) = line.split_once(':') else {
            repaired.push(line.to_string());
            continue;
        };
        if key.trim().is_empty() || !value.chars().next().is_none_or(char::is_whitespace) {
            repaired.push(line.to_string());
            continue;
        }
        let trimmed = value.trim_start();
        let leading = &value[..value.len() - trimmed.len()];
        let (scalar, comment) = split_trailing_comment(trimmed);
        let scalar = scalar.trim_end();
        let Some(first) = scalar.chars().next() else {
            repaired.push(line.to_string());
            continue;
        };
        if matches!(first, '|' | '>') {
            block_scalar_indent = Some(indent);
            repaired.push(line.to_string());
            continue;
        }
        if matches!(first, '\'' | '"') {
            repaired.push(line.to_string());
            continue;
        }
        let has_colon_separator = scalar.char_indices().any(|(index, character)| {
            character == ':'
                && scalar[index + character.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
        });
        let invalid_flow_scalar = matches!(first, '[' | '{' | '@' | '`' | '*' | '&')
            && serde_yaml::from_str::<serde_yaml::Value>(scalar).is_err();
        if !has_colon_separator && !invalid_flow_scalar {
            repaired.push(line.to_string());
            continue;
        }
        let quoted = format!("'{}'", scalar.replace('\'', "''"));
        repaired.push(format!("{key}:{leading}{quoted}{comment}"));
        changed = true;
    }
    changed.then(|| repaired.join("\n"))
}

fn split_trailing_comment(value: &str) -> (&str, &str) {
    for (index, character) in value.char_indices() {
        let starts_comment = character == '#'
            && (index == 0
                || value[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace));
        if starts_comment {
            let end = value[..index].trim_end().len();
            return (&value[..end], &value[end..]);
        }
    }
    (value, "")
}

#[cfg(test)]
mod tests;
