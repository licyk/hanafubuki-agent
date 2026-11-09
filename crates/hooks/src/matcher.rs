// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/hooks/src/engine/matcher.rs` (the matcher kinds) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: `matches` takes every name a call is known
// by (its own and its aliases).

/// A group's `matcher`, compiled when the hook is discovered: empty or `*`
/// matches everything, names joined by `|` match one of them exactly, and
/// anything else is a regular expression.
#[derive(Clone, Debug)]
pub(crate) enum HookMatcher {
    All,
    Exact(String),
    Regex(regex::Regex),
}

impl HookMatcher {
    pub(crate) fn new(pattern: &str) -> Result<Self, regex::Error> {
        if pattern.is_empty() || pattern == "*" {
            Ok(Self::All)
        } else if pattern
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '|')
        {
            Ok(Self::Exact(pattern.to_string()))
        } else {
            regex::Regex::new(pattern).map(Self::Regex)
        }
    }

    /// Whether any of `inputs` matches.
    pub(crate) fn matches(&self, inputs: &[&str]) -> bool {
        match self {
            Self::All => true,
            Self::Exact(pattern) => inputs
                .iter()
                .any(|input| pattern.split('|').any(|candidate| candidate == *input)),
            Self::Regex(regex) => inputs.iter().any(|input| regex.is_match(input)),
        }
    }
}
