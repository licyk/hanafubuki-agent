//! Mutating one user-owned hook preserves the other entries.
use super::AgentConfig;
use crate::file::resolve_destination;
use crate::file::{write_atomic, FileAccess};
use serde_json::{json, Value};
use std::fs;

impl AgentConfig {
    /// Updates a hook in the user's hooks.json or config.toml. The caller
    /// validates the replacement group against the hook definition schema.
    pub fn write_user_hook(
        &self,
        file: &str,
        previous: Option<(&str, usize, usize)>,
        replacement: Option<(&str, Value)>,
    ) -> Result<(), String> {
        if !matches!(file, "hooks.json" | "config.toml") {
            return Err("Not a user hook file".into());
        }
        let mut cached = self.inner.settings.lock();
        let directory = self
            .inner
            .settings_file
            .path()
            .parent()
            .ok_or("Configuration has no directory")?;
        let path = resolve_destination(&directory.join(file))?;
        let mut document = if file == "config.toml" {
            Value::Object(self.load_user_settings()?.document)
        } else {
            match fs::read(path.system()) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Read hooks.json: {error}"))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
                Err(error) => return Err(format!("Read hooks.json: {error}")),
            }
        };
        let object = document
            .as_object_mut()
            .ok_or("Hook file is not an object")?;
        let hooks = object
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("hooks is not an object")?;
        let original = previous.and_then(|(event, group, handler)| {
            let group = hooks
                .get(event)?
                .as_array()?
                .get(group)?
                .as_object()?
                .clone();
            let handler = group
                .get("hooks")?
                .as_array()?
                .get(handler)?
                .as_object()?
                .clone();
            Some((group, handler))
        });
        if let Some((event, group, handler)) = previous {
            let handlers = hooks
                .get_mut(event)
                .and_then(Value::as_array_mut)
                .and_then(|groups| groups.get_mut(group))
                .and_then(|group| group.get_mut("hooks"))
                .and_then(Value::as_array_mut)
                .ok_or("Hook changed; reload before saving")?;
            if handler >= handlers.len() {
                return Err("Hook changed; reload before saving".into());
            }
            handlers.remove(handler);
        }
        if let Some((event, mut group)) = replacement {
            if let Some((mut old_group, mut old_handler)) = original {
                let new_handler = group["hooks"][0]
                    .as_object()
                    .ok_or("Hook handler is not an object")?;
                for key in ["type", "command", "timeout", "statusMessage"] {
                    old_handler.remove(key);
                }
                old_handler.extend(new_handler.clone());
                if cfg!(windows)
                    && (old_handler.contains_key("commandWindows")
                        || old_handler.contains_key("command_windows"))
                {
                    old_handler.remove("command_windows");
                    old_handler.insert("commandWindows".into(), new_handler["command"].clone());
                }
                old_group.remove("matcher");
                if let Some(matcher) = group.get("matcher") {
                    old_group.insert("matcher".into(), matcher.clone());
                }
                old_group.insert("hooks".into(), json!([old_handler]));
                group = Value::Object(old_group);
            }
            hooks
                .entry(event)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or("Hook event is not a list")?
                .push(group);
        }
        if file == "config.toml" {
            self.inner.settings_file.write(&json!({"hooks": hooks}))?;
        } else {
            let bytes = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
            write_atomic(path.system(), &bytes, FileAccess::Private)?;
        }
        *cached = None;
        Ok(())
    }
}
