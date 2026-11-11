//! One role's table, read key by key: our camelCase keys, and Codex's
//! `developer_instructions`, `model` and `model_reasoning_effort`.

use std::collections::BTreeSet;

use hanafubuki_agent_foundation::agent_settings::{AgentPermissionMode, ModelReasoningEffort};
use serde_json::{Map, Value};

use super::{InstructionsMode, RoleDefinition, RoleOrigin};

/// Keys the caller reads: the file's name and an entry's role file.
const READ_BY_CALLER: [&str; 3] = ["name", "configFile", "config_file"];

/// The role `name` that `table` defines.
pub(super) fn decode_role(
    name: String,
    origin: RoleOrigin,
    table: &Map<String, Value>,
) -> RoleDefinition {
    let mut reader = Reader {
        table,
        read: READ_BY_CALLER.into_iter().collect(),
        role: RoleDefinition::empty(name, origin),
    };
    let description = reader.text(&["description"]);
    let instructions = reader.text(&["instructions", "developer_instructions"]);
    let instructions_mode =
        reader.parsed(
            &["instructionsMode"],
            "append or replace",
            |mode| match mode {
                "append" => Some(InstructionsMode::Append),
                "replace" => Some(InstructionsMode::Replace),
                _ => None,
            },
        );
    let model = reader.text(&["model"]);
    let reasoning_effort = reader.parsed(
        &["reasoningEffort", "model_reasoning_effort"],
        "a reasoning effort",
        |effort| ModelReasoningEffort::parse(effort).ok(),
    );
    let spawnable = reader.boolean("spawnable");
    let extends = reader.narrowing_text("extends");
    let permission_mode_cap = reader.narrowing_mode("permissionModeCap");
    let (tools_allow, tools_deny) = reader.tools();
    let plugins_off = reader.plugins_off();
    reader.unknown_keys();
    let mut role = reader.role;
    role.description = description;
    role.instructions = instructions;
    role.instructions_mode = instructions_mode.unwrap_or_default();
    role.model = model;
    role.reasoning_effort = reasoning_effort;
    role.spawnable = spawnable;
    role.extends = extends;
    role.permission_mode_cap = permission_mode_cap;
    role.tools_allow = tools_allow;
    role.tools_deny = tools_deny;
    role.plugins_off = plugins_off;
    role
}

struct Reader<'a> {
    table: &'a Map<String, Value>,
    read: BTreeSet<&'static str>,
    role: RoleDefinition,
}

impl<'a> Reader<'a> {
    /// The value of the first of `keys` (ours, then Codex's alias) that is
    /// set, with its key.
    fn value(&mut self, keys: &[&'static str]) -> Option<(&'static str, &'a Value)> {
        self.read.extend(keys.iter().copied());
        let table = self.table;
        let mut set = keys.iter().filter_map(|key| Some((*key, table.get(*key)?)));
        let first = set.next()?;
        for (alias, _) in set {
            self.role.issues.push(format!(
                "`{alias}` is ignored: `{}` is also set, and is used",
                first.0
            ));
        }
        Some(first)
    }

    /// Text that is not blank.
    fn text(&mut self, keys: &[&'static str]) -> Option<String> {
        let (key, value) = self.value(keys)?;
        match value.as_str().map(str::trim) {
            Some("") => {
                self.ignored(key, "is empty");
                None
            }
            Some(_) => value.as_str().map(str::to_string),
            None => {
                self.ignored(key, "is not text");
                None
            }
        }
    }

    fn parsed<T>(
        &mut self,
        keys: &[&'static str],
        expected: &str,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> Option<T> {
        let (key, value) = self.value(keys)?;
        let parsed = value.as_str().and_then(parse);
        if parsed.is_none() {
            self.ignored(key, &format!("is not {expected}"));
        }
        parsed
    }

    fn boolean(&mut self, key: &'static str) -> Option<bool> {
        let (key, value) = self.value(&[key])?;
        let parsed = value.as_bool();
        if parsed.is_none() {
            self.ignored(key, "is not true or false");
        }
        parsed
    }

    fn narrowing_text(&mut self, key: &'static str) -> Option<String> {
        let (key, value) = self.value(&[key])?;
        match value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            Some(text) => Some(text.to_string()),
            None => {
                self.unusable(key, "is not a role's name");
                None
            }
        }
    }

    fn narrowing_mode(&mut self, key: &'static str) -> Option<AgentPermissionMode> {
        let (key, value) = self.value(&[key])?;
        let mode = serde_json::from_value::<AgentPermissionMode>(value.clone()).ok();
        if mode.is_none() {
            self.unusable(key, "is not manual, auto_review or full_access");
        }
        mode
    }

    /// `[tools]`: `allow` and `deny`.
    fn tools(&mut self) -> (Option<Vec<String>>, Vec<String>) {
        let Some(table) = self.narrowing_table("tools") else {
            return (None, Vec::new());
        };
        let allow = table
            .get("allow")
            .and_then(|allow| self.patterns("tools.allow", allow));
        let deny = table
            .get("deny")
            .and_then(|deny| self.patterns("tools.deny", deny))
            .unwrap_or_default();
        self.unknown_in(&table, "tools", &["allow", "deny"]);
        (allow, deny)
    }

    /// `[plugins]`: `off`.
    fn plugins_off(&mut self) -> Vec<String> {
        let Some(table) = self.narrowing_table("plugins") else {
            return Vec::new();
        };
        let off = table
            .get("off")
            .and_then(|off| self.patterns("plugins.off", off))
            .unwrap_or_default();
        self.unknown_in(&table, "plugins", &["off"]);
        off
    }

    fn narrowing_table(&mut self, key: &'static str) -> Option<Map<String, Value>> {
        let (key, value) = self.value(&[key])?;
        match value.as_object() {
            Some(table) => Some(table.clone()),
            None => {
                self.unusable(key, "is not a table");
                None
            }
        }
    }

    /// A list of strings; an item that is not one is left out alone, which
    /// leaves a narrowing list no wider than written.
    fn patterns(&mut self, key: &str, value: &Value) -> Option<Vec<String>> {
        let Some(items) = value.as_array() else {
            self.unusable(key, "is not a list");
            return None;
        };
        let mut kept = Vec::new();
        for (index, item) in items.iter().enumerate() {
            match item.as_str() {
                Some(text) => kept.push(text.to_string()),
                None => self
                    .role
                    .issues
                    .push(format!("`{key}[{index}]` is not text; it is left out")),
            }
        }
        Some(kept)
    }

    fn unknown_in(&mut self, table: &Map<String, Value>, prefix: &str, known: &[&str]) {
        for key in table.keys().filter(|key| !known.contains(&key.as_str())) {
            self.role
                .issues
                .push(format!("`{prefix}.{key}` is not a role key; it is ignored"));
        }
    }

    /// Every key nothing read: a Codex setting is process-wide here.
    fn unknown_keys(&mut self) {
        for key in self.table.keys() {
            if !self.read.contains(key.as_str()) {
                self.role.issues.push(format!(
                    "`{key}` is not a role key (a Codex setting is process-wide here); it is \
                     ignored"
                ));
            }
        }
    }

    fn ignored(&mut self, key: &str, why: &str) {
        self.role
            .issues
            .push(format!("`{key}` {why}; it is ignored"));
    }

    fn unusable(&mut self, key: &str, why: &str) {
        if self.role.unusable.is_none() {
            self.role.unusable = Some(format!(
                "`{key}` {why}, and leaving it out would allow more than the role says"
            ));
        }
    }
}
