// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/rules.ts` of acp-kernel 0.0.189 and `executeRule` of
// `src/rules-feature.ts`, at commit `4be21b08e`. Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the limits come from the configuration
// alone; `execute_rule` returns its outcome instead of logging it.

use serde_json::Value;

use crate::model::{Config, RuleRecord, State};
use crate::text::utf16_len;

/// The system-prompt section that explains `acp_rule` while rules are on.
pub const RULES_USAGE_PROMPT: &str = "Use the acp_rule tool to record short, principle-level reminders that must survive context compression:\n- behavioral corrections the user has had to repeat more than once,\n- project invariants the user explicitly asked you to remember,\n- pitfalls you ran into once and must not run into again.\nRules are re-injected into the system prompt every turn. Omit the text argument to list recorded rules.";

/// The id the next recorded rule gets. Ids are never reissued: the counter
/// only grows, and a hand-made state's highest id moves it past.
pub fn allocate_rule_id(state: &State) -> String {
    format!("rule{}", next_rule_number(state))
}

fn next_rule_number(state: &State) -> u32 {
    let highest = state
        .rules
        .iter()
        .filter_map(|rule| rule.id.strip_prefix("rule"))
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .filter_map(|digits| digits.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    state.next_rule_id.max(highest + 1)
}

/// Records a rule; the error tells the model how to fix its input.
pub fn add_rule(state: &mut State, text: &str, config: &Config) -> Result<RuleRecord, String> {
    let limits = &config.rules;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("rule text is empty — provide the reminder to record.".to_owned());
    }
    let length = utf16_len(trimmed);
    if length > limits.max_rule_chars {
        return Err(format!(
            "{length} chars exceeds the {}-char limit — keep rules short and principle-level.",
            limits.max_rule_chars
        ));
    }
    if let Some(duplicate) = state.rules.iter().find(|rule| rule.text == trimmed) {
        return Err(format!(
            "identical rule already exists ({}) — no change.",
            duplicate.id
        ));
    }
    if state.rules.len() >= limits.max_rules {
        return Err(format!(
            "rule limit reached ({}) — remove or clear outdated rules first.",
            limits.max_rules
        ));
    }
    let number = next_rule_number(state);
    let rule = RuleRecord {
        id: format!("rule{number}"),
        text: trimmed.to_owned(),
    };
    state.rules.push(rule.clone());
    state.next_rule_id = number + 1;
    Ok(rule)
}

pub fn remove_rule(state: &mut State, id: &str) -> Result<RuleRecord, String> {
    let id = id.trim();
    let Some(index) = state.rules.iter().position(|rule| rule.id == id) else {
        return Err(format!(
            "no rule with id \"{id}\" — list current rules first (omit the text argument)."
        ));
    };
    Ok(state.rules.remove(index))
}

/// Removes every rule; the count removed.
pub fn clear_rules(state: &mut State) -> usize {
    std::mem::take(&mut state.rules).len()
}

/// The system-prompt section listing the rules; empty when there are none,
/// so a session without rules pays nothing.
pub fn format_rules_for_prompt(state: &State) -> String {
    if state.rules.is_empty() {
        return String::new();
    }
    let mut lines =
        vec!["# Persistent rules (recorded via acp_rule — kept across compression)".to_owned()];
    lines.extend(
        state
            .rules
            .iter()
            .map(|rule| format!("- [{}] {}", rule.id, rule.text)),
    );
    lines.join("\n")
}

/// The numbered list `acp_rule` answers a listing with.
pub fn format_rules_list(rules: &[RuleRecord]) -> String {
    rules
        .iter()
        .enumerate()
        .map(|(index, rule)| format!("{}. [{}] {}", index + 1, rule.id, rule.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The tool result of one `acp_rule` call; `ok` is false for a refusal,
/// which is still a normal answer telling the model what to fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleToolResult {
    pub ok: bool,
    pub text: String,
}

/// Runs one `acp_rule` call on the state: `rule` records, `delete` removes
/// one, `clear: true` removes all, and no argument lists them; more than one
/// operation is refused.
pub fn execute_rule(arguments: &Value, state: &mut State, config: &Config) -> RuleToolResult {
    let string = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map_or("", str::trim)
    };
    let delete = string("delete");
    let rule = string("rule");
    let clear = arguments.get("clear") == Some(&Value::Bool(true));
    let operations = [!delete.is_empty(), clear, !rule.is_empty()]
        .iter()
        .filter(|operation| **operation)
        .count();
    let outcome = if operations > 1 {
        Err("Use one operation per call: record (rule), remove one (delete), remove all (clear: true), or list (no arguments).".to_owned())
    } else if clear {
        Ok(match clear_rules(state) {
            0 => "No rules to clear.".to_owned(),
            count => format!("Cleared {count} rule(s)."),
        })
    } else if !delete.is_empty() {
        remove_rule(state, delete).map(|rule| format!("Removed {}: {}", rule.id, rule.text))
    } else if rule.is_empty() {
        Ok(if state.rules.is_empty() {
            "No rules recorded.".to_owned()
        } else {
            format_rules_list(&state.rules)
        })
    } else {
        add_rule(state, rule, config).map(|rule| format!("Recorded {}: {}", rule.id, rule.text))
    };
    match outcome {
        Ok(text) => RuleToolResult { ok: true, text },
        Err(text) => RuleToolResult { ok: false, text },
    }
}
