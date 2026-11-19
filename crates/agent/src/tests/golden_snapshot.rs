use std::path::Path;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use serde_json::{Map, Value};

pub(super) fn normalize(value: &Value, app_data_dir: &Path) -> Value {
    // Some request paths have been resolved through a symlink in the temporary directory.
    let resolved = ResolvedPath::resolve(app_data_dir).unwrap();
    normalize_host(
        value,
        [resolved.native_text(), &app_data_dir.to_string_lossy()],
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

fn normalize_host(value: &Value, paths: [&str; 2], os: &str, arch: &str) -> Value {
    let mut roots = Vec::new();
    for path in paths {
        roots.push(path.to_string());
        let quoted = serde_json::to_string(path).unwrap();
        roots.push(quoted[1..quoted.len() - 1].to_string());
    }
    roots.sort_by_key(|root| std::cmp::Reverse(root.len()));
    roots.dedup();
    let replace = |text: &str| {
        let mut text = text.to_owned();
        for root in &roots {
            text = text.replace(root, "<APP_DATA>");
        }
        if os == "windows" {
            text = normalize_fixture_separators(&text);
        }
        let text = text
            .replace(
                &format!(r#""architecture":"{arch}""#),
                r#""architecture":"<ARCH>""#,
            )
            .replace(&format!(r#""platform":"{os}""#), r#""platform":"<OS>""#);
        redact_digests(&text)
    };
    fn walk(value: &Value, replace: &dyn Fn(&str) -> String) -> Value {
        match value {
            Value::String(text) => Value::String(replace(text)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| walk(item, replace)).collect())
            }
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), walk(value, replace)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    sorted(&walk(value, &replace))
}

/// World-state sections carry a content digest that covers temporary paths,
/// and the tool catalog revision covers each MCP connection's generation.
fn redact_digests(text: &str) -> String {
    ["version=\"", "\"catalogRevision\":\""]
        .into_iter()
        .fold(text.to_string(), |text, marker| {
            redact_digests_after(&text, marker)
        })
}

fn redact_digests_after(text: &str, marker: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find(marker) {
        let start = position + marker.len();
        out.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let digest_length = candidate
            .bytes()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if digest_length == 64 {
            out.push_str("<DIGEST>");
            rest = &candidate[64..];
        } else {
            rest = candidate;
        }
    }
    out.push_str(rest);
    out
}

/// Object keys in a stable order whether or not `serde_json/preserve_order` is
/// enabled somewhere in the dependency graph.
pub(super) fn sorted(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), sorted(&object[key]));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Paths occur in prose and in JSON embedded in request strings.
fn normalize_fixture_separators(text: &str) -> String {
    let mut parts = text.split("<APP_DATA>");
    let mut result = parts.next().unwrap().to_owned();
    for part in parts {
        result.push_str("<APP_DATA>");
        let end = part
            .find(['\n', '\r', '"', '`', '<', '>'])
            .unwrap_or(part.len());
        let (path, rest) = part.split_at(end);
        result.push_str(&path.replace(r"\\", "/").replace('\\', "/"));
        result.push_str(rest);
    }
    result
}

#[test]
fn host_paths_and_environment_have_one_snapshot() {
    use serde_json::json;
    for (os, arch, native, lexical, sep) in [
        ("linux", "x86_64", "/tmp/work", "/tmp/work", "/"),
        (
            "macos",
            "aarch64",
            "/private/var/tmp/work",
            "/var/tmp/work",
            "/",
        ),
        (
            "windows",
            "aarch64",
            r"C:\Temp Files\work",
            r"C:\Temp Files\work",
            r"\",
        ),
    ] {
        let roots = json!({"roots": [format!("{native}{sep}instances")]}).to_string();
        let request = json!([
            format!("<agent_world_state>\n{roots}\n</agent_world_state>"),
            format!("source=\"{lexical}{sep}instances{sep}AGENTS.md\""),
            json!({"architecture": arch, "platform": os}).to_string(),
            r"keep\this",
        ]);
        assert_eq!(
            normalize_host(&request, [native, lexical], os, arch),
            json!([
                "<agent_world_state>\n{\"roots\":[\"<APP_DATA>/instances\"]}\n</agent_world_state>",
                "source=\"<APP_DATA>/instances/AGENTS.md\"",
                r#"{"architecture":"<ARCH>","platform":"<OS>"}"#,
                r"keep\this",
            ])
        );
    }
}
