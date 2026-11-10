//! A code package checked as the guest host would run it, without starting
//! anything: its runtime's command or module, its contributions and its
//! settings' schema (`plugins validate`).

use std::path::Path;

use hanafubuki_agent_packages::{
    PackageCode, PackageContributes, PackageRuntime, PluginId, WasmRuntime,
};
use hanafubuki_agent_tool_contracts::validate_json_schema;
use serde_json::Value;

use crate::contribute::check_contributes;
use crate::spawn::{in_package, interpreter, resolve};

/// The protocol the guest host offers a guest.
const PROTOCOL: &str = "kardia-guest/2";

/// What would keep the code of the package `id` at `root` from starting,
/// or leave a declaration of it out, each as an issue.
pub fn check_package(id: &PluginId, root: &Path, code: &PackageCode) -> Vec<String> {
    let mut issues = Vec::new();
    match &code.runtime {
        None => issues.push(
            "it declares code (contributes, permissions or configSchema) but no usable runtime, \
             so nothing runs"
                .to_string(),
        ),
        Some(PackageRuntime::Wasm(runtime)) => {
            issues.extend(module_issue(root, &runtime.module));
            issues.extend(time_issues(runtime, &code.contributes));
        }
        Some(PackageRuntime::Process(runtime)) => {
            if let Some(protocol) = runtime.protocol.as_deref().filter(|p| *p != PROTOCOL) {
                issues.push(format!(
                    "runtime.protocol `{protocol}` is not the one the Agent speaks ({PROTOCOL})"
                ));
            }
            issues.extend(command_issues(root, &runtime.command, runtime.interpreter));
        }
    }
    issues.extend(check_contributes(id, &code.contributes));
    if let Some(schema) = &code.config_schema {
        let schema = Value::Object(schema.clone());
        if validate_json_schema(&schema).is_err() {
            issues.push("its configSchema is too large or too deep".to_string());
        }
        if schema
            .get("type")
            .is_some_and(|kind| kind != &Value::String("object".into()))
        {
            issues.push("its configSchema must describe an object: settings are one".to_string());
        }
    }
    issues
}

/// What keeps the module from loading: a path that is not in the package,
/// or a file that is not a binary WebAssembly module.
fn module_issue(root: &Path, module: &str) -> Option<String> {
    let found = resolve(root, "its package directory").and_then(|root| in_package(&root, module));
    let path = match found {
        Ok(path) => path,
        Err(error) => return Some(format!("runtime.module: {error}")),
    };
    let mut magic = [0; 4];
    let read = std::fs::File::open(path.system())
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut magic));
    match read {
        Ok(()) if magic == *b"\0asm" => None,
        Ok(()) | Err(_) => Some(format!(
            "runtime.module: `{module}` is not a WebAssembly module (a binary .wasm file)"
        )),
    }
}

/// The tools whose `timeout` a shorter `limits.timeoutMs` cuts.
fn time_issues(runtime: &WasmRuntime, contributes: &PackageContributes) -> Vec<String> {
    let Some(limit) = runtime.limits.timeout else {
        return Vec::new();
    };
    contributes
        .tools
        .iter()
        .filter(|tool| tool.timeout.is_some_and(|timeout| timeout > limit))
        .map(|tool| {
            format!(
                "tool `{}`: its timeout is longer than runtime.limits.timeoutMs ({} ms), which \
                 stops its calls first",
                tool.name,
                limit.as_millis()
            )
        })
        .collect()
}

/// What keeps `command` from starting: a program in the package
/// (`./…`) that is not there, or an interpreter not on the `PATH`, and an
/// argument in the package that is not there.
fn command_issues(root: &Path, command: &[String], interpreted: bool) -> Vec<String> {
    let root = match resolve(root, "its package directory") {
        Ok(root) => root,
        Err(error) => return vec![error],
    };
    let mut issues = Vec::new();
    for (index, part) in command.iter().enumerate() {
        let found = match (index, interpreted) {
            (0, true) => interpreter(part),
            (_, _) if index == 0 || part.starts_with("./") => in_package(&root, part),
            _ => continue,
        };
        match found {
            Ok(path) if index == 0 && !interpreted => {
                if let Some(issue) = not_executable(path.system(), part) {
                    issues.push(issue);
                }
            }
            Ok(_) => {}
            Err(error) => issues.push(format!("runtime.command: {error}")),
        }
    }
    issues
}

#[cfg(unix)]
fn not_executable(path: &Path, shown: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode();
    (mode & 0o111 == 0).then(|| format!("runtime.command: `{shown}` is not executable"))
}

#[cfg(not(unix))]
fn not_executable(_path: &Path, _shown: &str) -> Option<String> {
    None
}
