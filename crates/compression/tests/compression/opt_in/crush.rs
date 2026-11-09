// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the strategy, dispatch and valuation cases of `crush.test.ts`. Not
// ported: the plugin registry and the contract guards of host plugins
// (throwing, malformed or identity plugins); there are only the built-in
// strategies. The strategies' outputs were also checked byte for byte
// against the kernel's own code over a generated corpus while porting.

use serde_json::{json, Map, Value};

use hanafubuki_agent_compression::{
    classify_crush_text, crush_json, crush_text, evaluate_tool_result, Config, CrushConfig,
    CrushDecision, CrushKind, CrushStrategy, CrushStrategyOverride, CRUSH_STRATEGIES,
};

/// Expands the `__acp_crush` envelopes back into the document.
pub fn decode(value: &Value) -> Value {
    match value {
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                match item.get("__acp_crush").and_then(Value::as_str) {
                    Some("identical-run") => {
                        let count = item["count"].as_u64().unwrap_or(0);
                        out.extend((0..count).map(|_| decode(&item["item"])));
                    }
                    _ => out.push(decode(item)),
                }
            }
            Value::Array(out)
        }
        Value::Object(object) => match object.get("__acp_crush").and_then(Value::as_str) {
            Some("identical-run") => decode(&json!([value])),
            Some("rows") => {
                let rows = object["rows"].as_u64().unwrap_or(0) as usize;
                let constant = object["const"].as_object().cloned().unwrap_or_default();
                let items = object.get("items").and_then(Value::as_array);
                let rows = (0..rows).map(|index| {
                    let mut row: Map<String, Value> = constant.clone();
                    if let Some(Value::Object(varying)) = items.and_then(|items| items.get(index)) {
                        row.extend(varying.clone());
                    }
                    decode(&Value::Object(row))
                });
                Value::Array(rows.collect())
            }
            _ => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), decode(value)))
                    .collect(),
            ),
        },
        other => other.clone(),
    }
}

/// Written by hand: `json!` would sort the keys.
pub fn service_rows(count: usize) -> String {
    let rows: Vec<String> = (0..count)
        .map(|index| {
            format!(
                r#"{{"id":{},"ts":"2026-09-21T12:00:00Z","level":"INFO","name":"svc-{index}"}}"#,
                index * 7
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
}

fn fold(text: &str) -> Option<String> {
    crush_json(text)
}

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn crush(text: &str) -> Option<hanafubuki_agent_compression::CrushOutput> {
    crush_text(text, &CrushConfig::default(), None)
}

fn with_min_reduction(min_reduction: f64) -> CrushConfig {
    CrushConfig {
        min_reduction,
        ..CrushConfig::default()
    }
}

#[test]
fn constant_fields_are_hoisted_losslessly() {
    let source = service_rows(8);
    let out = fold(&source).expect("crushed");
    assert!(out.len() < source.len());
    let envelope = parsed(&out);
    assert_eq!(envelope["__acp_crush"], "rows");
    assert_eq!(envelope["rows"], 8);
    assert_eq!(envelope["const"]["ts"], "2026-09-21T12:00:00Z");
    assert_eq!(envelope["const"]["level"], "INFO");
    assert!(
        envelope["const"].get("id").is_none(),
        "a varying field stays"
    );
    assert_eq!(decode(&envelope), parsed(&source));
    assert_eq!(
        out,
        r#"{"__acp_crush":"rows","rows":8,"const":{"ts":"2026-09-21T12:00:00Z","level":"INFO"},"items":[{"id":0,"name":"svc-0"},{"id":7,"name":"svc-1"},{"id":14,"name":"svc-2"},{"id":21,"name":"svc-3"},{"id":28,"name":"svc-4"},{"id":35,"name":"svc-5"},{"id":42,"name":"svc-6"},{"id":49,"name":"svc-7"}]}"#
    );
}

#[test]
fn identical_runs_fold_losslessly() {
    let mut items = vec![json!({"e": "boom", "code": 500}); 5];
    items.extend([
        json!({"e": "ok", "code": 200}),
        json!({"e": "ok", "code": 200}),
    ]);
    items.extend([
        json!({"e": "warn", "code": 300}),
        json!({"e": "warn", "code": 300}),
    ]);
    let source = Value::Array(items).to_string();
    let out = fold(&source).expect("crushed");
    assert!(out.len() < source.len());
    assert_eq!(decode(&parsed(&out)), parsed(&source));
}

#[test]
fn an_all_identical_array_collapses() {
    let source =
        Value::Array(vec![json!({"status": "healthy", "region": "us-east-1"}); 6]).to_string();
    let out = fold(&source).expect("crushed");
    assert!(out.len() < source.len() / 2);
    assert_eq!(decode(&parsed(&out)), parsed(&source));
}

#[test]
fn folding_is_deterministic() {
    let rows: Vec<Value> = (0..10)
        .map(|index| json!({"id": index, "host": format!("h{index}"), "zone": "z1"}))
        .collect();
    let source = Value::Array(rows).to_string();
    assert_eq!(fold(&source), fold(&source));
}

#[test]
fn nested_envelopes_recurse() {
    let logs: Vec<Value> = (0..6)
        .map(|index| json!({"seq": index, "src": "api", "msg": format!("m{index}")}))
        .collect();
    let source = json!({"meta": "x", "logs": logs}).to_string();
    let out = parsed(&fold(&source).expect("crushed"));
    assert_eq!(out["logs"]["__acp_crush"], "rows");
    assert_eq!(decode(&out), parsed(&source));
}

#[test]
fn nothing_redundant_is_never_reformatted() {
    let rows: Vec<Value> = (0..8)
        .map(|index| json!({"id": index, "a": format!("a{index}"), "b": format!("b{index}"), "c": index % 3}))
        .collect();
    assert_eq!(fold(&Value::Array(rows).to_string()), None);
    assert_eq!(fold(r#"{"a":1,"b":[1,2,3]}"#), None);
}

#[test]
fn numbers_are_written_as_javascript_writes_them() {
    let source = r#"[{"v":1.0,"w":1e21,"x":0.0000001},{"v":1.0,"w":1e21,"x":0.0000001},{"v":1.0,"w":1e21,"x":0.0000001},{"v":1.0,"w":1e21,"x":0.0000001}]"#;
    assert_eq!(
        fold(source).as_deref(),
        Some(r#"{"__acp_crush":"rows","rows":4,"const":{"v":1,"w":1e+21,"x":1e-7}}"#)
    );
}

#[test]
fn a_key_missing_from_the_first_row_is_never_hoisted() {
    let rows: Vec<Value> = (0..60)
        .map(|index| {
            let mut row = json!({"zone": "z1", "id": format!("id-{index:03}"), "name": format!("{:.<90}", format!("item-{index}"))});
            if index > 0 {
                row["a"] = Value::Null;
            }
            row
        })
        .collect();
    let source = Value::Array(rows).to_string();
    let envelope = parsed(&fold(&source).expect("crushed"));
    assert_eq!(envelope["__acp_crush"], "rows");
    assert_eq!(envelope["const"]["zone"], "z1");
    assert_eq!(decode(&envelope), parsed(&source));
}

#[test]
fn malformed_json_and_prose_pass_through() {
    assert_eq!(crush("[1,2,"), None);
    assert_eq!(
        crush(&"The quick brown fox jumps over the lazy dog. ".repeat(20)),
        None
    );
    assert_eq!(crush(""), None);
}

#[test]
fn the_minimum_reduction_gates_marginal_savings() {
    let source = Value::Array(vec![json!("x".repeat(100)); 4]).to_string();
    assert!(crush_text(&source, &with_min_reduction(0.5), None).is_some());
    assert_eq!(crush_text(&source, &with_min_reduction(0.9), None), None);
}

#[test]
fn json_dispatches_first_then_code() {
    let rows: Vec<Value> = (0..8)
        .map(|index| json!({"id": index * 3, "zone": "z1"}))
        .collect();
    assert_eq!(
        crush(&Value::Array(rows).to_string()).unwrap().strategy,
        CrushStrategy::JsonFold
    );
    let python =
        "import os\n\ndef f():\n    # c1\n    # c2\n    # c3\n    # c4\n    return os.getcwd()";
    assert_eq!(crush(python).unwrap().strategy, CrushStrategy::CodeTrim);
}

#[test]
fn texts_are_classified_json_code_or_log() {
    assert_eq!(classify_crush_text(r#"[{"a":1}]"#), CrushKind::Json);
    assert_eq!(
        classify_crush_text("import os\n\ndef f():\n    pass"),
        CrushKind::Code
    );
    let ticks: Vec<String> = (0..60).map(|index| format!("INFO tick {index}")).collect();
    assert_eq!(classify_crush_text(&ticks.join("\n")), CrushKind::Log);
    assert_eq!(
        classify_crush_text("plain english paragraph without structure"),
        CrushKind::Log
    );
}

#[test]
fn the_defaults_are_the_kernels() {
    let defaults = Config::new(200_000).crush;
    assert!(!defaults.enabled);
    assert_eq!(defaults.min_reduction, 0.1);
    assert!(defaults.strategies.is_empty());
    let ids: Vec<&str> = CRUSH_STRATEGIES
        .iter()
        .map(|strategy| strategy.id())
        .collect();
    assert_eq!(ids, ["json-fold", "code-trim", "log-select"]);
}

#[test]
fn the_minimum_reduction_is_validated() {
    for (value, valid) in [(0.5, true), (0.0, false), (1.5, false), (f64::NAN, false)] {
        let mut config = Config::new(200_000);
        config.crush.min_reduction = value;
        let expected: Vec<&str> = if valid {
            vec![]
        } else {
            vec!["crush.minReduction must be in (0, 1]"]
        };
        assert_eq!(config.validate(), expected, "{value}");
    }
}

// --- evaluateToolResult ---

fn gates(min_tool_tokens: u64) -> Config {
    let mut config = Config::new(200_000);
    config.absorb.enabled = true;
    config.absorb.min_tool_tokens = min_tool_tokens;
    config.crush.enabled = true;
    config
}

#[test]
fn under_the_gates_a_result_is_skipped() {
    let small = r#"[{"id":1}]"#;
    let skipped = evaluate_tool_result(small, &gates(100), 0, None);
    assert_eq!(skipped.decision, CrushDecision::Skip);
    assert_eq!(skipped.text, small);
    let mut pressure = gates(4000);
    pressure.model_context_limit = 100_000;
    pressure.absorb.context_threshold_pct = 0.8;
    let idle = evaluate_tool_result(&service_rows(10), &pressure, 1000, None);
    assert_eq!(idle.decision, CrushDecision::Skip);
}

#[test]
fn a_result_crushed_under_the_gate_is_done() {
    let evaluation = evaluate_tool_result(&service_rows(10), &gates(100), 0, None);
    assert_eq!(evaluation.decision, CrushDecision::Crushed);
    assert!(evaluation.new_tokens.is_some_and(|tokens| tokens < 100));
    assert!(evaluation
        .reduction
        .is_some_and(|reduction| reduction > 0.0));
    assert_eq!(evaluation.strategy, Some(CrushStrategy::JsonFold));
    assert!(evaluation.text.contains("__acp_crush"));
}

pub fn request_rows() -> String {
    let rows: Vec<Value> = (0..40)
        .map(|index| json!({"id": index, "level": "INFO", "msg": format!("request handled with status code and correlation value {index}")}))
        .collect();
    Value::Array(rows).to_string()
}

#[test]
fn still_over_the_gate_the_crushed_text_is_distilled() {
    let source = request_rows();
    let evaluation = evaluate_tool_result(&source, &gates(100), 0, None);
    assert_eq!(evaluation.decision, CrushDecision::Distill);
    assert!(evaluation.text.contains("__acp_crush"));
}

#[test]
fn an_uncrushable_result_is_distilled_as_it_is() {
    let prose = "Sentence about ordinary topics. ".repeat(200);
    let evaluation = evaluate_tool_result(&prose, &gates(100), 0, None);
    assert_eq!(evaluation.decision, CrushDecision::Distill);
    assert_eq!(evaluation.text, prose);
}

#[test]
fn a_strategy_can_be_turned_off_or_exclude_tools() {
    let source = service_rows(10);
    let mut off = gates(100);
    off.crush.strategies.insert(
        "json-fold".to_owned(),
        CrushStrategyOverride {
            enabled: false,
            ..CrushStrategyOverride::default()
        },
    );
    let evaluation = evaluate_tool_result(&source, &off, 0, None);
    assert_eq!(
        (evaluation.decision, evaluation.text),
        (CrushDecision::Distill, source)
    );
    let code = "import os\n\ndef f():\n    # c1\n    # c2\n    # c3\n    # c4\n    # c5\n    # c6\n    return os.getcwd()\n".repeat(30);
    let mut excluding = gates(100);
    excluding.crush.strategies.insert(
        "code-trim".to_owned(),
        CrushStrategyOverride {
            exclude_tools: vec!["bash".to_owned()],
            ..CrushStrategyOverride::default()
        },
    );
    assert_eq!(
        evaluate_tool_result(&code, &excluding, 0, Some("bash")).text,
        code
    );
    assert_ne!(
        evaluate_tool_result(&code, &excluding, 0, Some("python")).text,
        code
    );
}

// --- code-trim ---

const PY_DOCSTRING: &str = "#!/usr/bin/env python3\n# module-level comment\n\n\ndef fetch(url):\n    \"\"\"Fetch the url.\n\n        Multi-line body.\n    \"\"\"\n    import urllib.request\n    return urllib.request.urlopen(url).read()";

#[test]
fn python_docstrings_and_comments_are_elided() {
    let out = CrushStrategy::CodeTrim.run(PY_DOCSTRING).expect("crushed");
    assert_eq!(
        out,
        "#!/usr/bin/env python3\n# [acp-crush: elided 1 line]\ndef fetch(url):\n# [acp-crush: elided 3 lines]\n    import urllib.request\n    return urllib.request.urlopen(url).read()"
    );
}

#[test]
fn an_assigned_triple_quoted_string_is_kept() {
    let source = "import os\n# comment one\n# comment two\n# comment three\n# comment four\n# comment five\n\ndef gen():\n    \"\"\"Docstring body.\n        second line.\n    \"\"\"\n    x = \"\"\"kept string\"\"\"\n    return x";
    let out = CrushStrategy::CodeTrim.run(source).expect("crushed");
    assert!(out.contains("x = \"\"\"kept string\"\"\""));
    assert!(!out.contains("Docstring body"));
}

#[test]
fn an_unterminated_triple_quote_fails_open() {
    let source = "import os\n\ndef f():\n    \"\"\"unclosed docstring\n    return x";
    assert_eq!(CrushStrategy::CodeTrim.run(source), None);
}

#[test]
fn javascript_comments_are_elided_and_templates_kept() {
    let source = "// top comment one\n// top comment two\n// top comment three\n// top comment four\nfunction greet(name) {\n  // build a safe url\n  const url = `https://x/${name.split(\"//\")[0]}/y`;\n  /* block\n     comment line two\n     comment */\n  console.log(`hi ${url}`);\n  return url;\n}";
    let out = CrushStrategy::CodeTrim.run(source).expect("crushed");
    assert_eq!(
        out,
        "// [acp-crush: elided 4 lines]\nfunction greet(name) {\n// [acp-crush: elided 1 line]\n  const url = `https://x/${name.split(\"//\")[0]}/y`;\n// [acp-crush: elided 3 lines]\n  console.log(`hi ${url}`);\n  return url;\n}"
    );
}

#[test]
fn prose_is_not_code() {
    assert_eq!(
        CrushStrategy::CodeTrim.run(&"This is a plain English paragraph. ".repeat(30)),
        None
    );
}

#[test]
fn a_regex_literal_opens_no_comment() {
    let source = "function f() {\n  var re = /[/*]/;\n  var a = 1;\n  var b = 2;\n  var c = 3;\n  var d = 4;\n  var s = \"has */ inside\";\n  var e = 5;\n  var g = 6;\n  return a + b + c + d + e + g;\n}";
    if let Some(out) = crush(source) {
        for line in [
            "var re = /[/*]/;",
            "var a = 1;",
            "var d = 4;",
            "var e = 5;",
            "var g = 6;",
        ] {
            assert!(out.text.contains(line), "{line}");
        }
    }
}

#[test]
fn division_is_not_a_regex() {
    let source = "function ratio(total, parts) {\n  var a = total / parts[0];\n  var b = (total + 1) / (parts[1] || 1);\n  var c = -total / parts[2];\n  /* first full-line comment */\n  /* second full-line comment */\n  var d = total / 2 * parts[3];\n  return a + b + c + d;\n}";
    let out = crush(source).expect("crushed").text;
    assert!(out.contains("var a = total / parts[0];"));
    assert!(out.contains("var b = (total + 1) / (parts[1] || 1);"));
    assert!(out.contains("var d = total / 2 * parts[3];"));
    assert!(!out.contains("first full-line comment"));
}

#[test]
fn after_a_quote_the_kernels_scanner_sees_nothing() {
    // The kernel reopens a string at its closing quote: a block comment
    // opened after a string on its line is not seen, and its lines stay.
    let source = "const a = 1;\nconst b = 2;\nfunction f() {\n  foo(\"x\"); /* open\n  still comment\n  more comment\n  */\n  // c1\n  // c2\n  // c3\n  // c4\n  // c5\n  // c6\n  return 1;\n}\n";
    let out = CrushStrategy::CodeTrim.run(source).expect("crushed");
    assert_eq!(
        out,
        "const a = 1;\nconst b = 2;\nfunction f() {\n  foo(\"x\"); /* open\n  still comment\n  more comment\n  */\n// [acp-crush: elided 6 lines]\n  return 1;\n}\n"
    );
}

// --- log-select ---

pub fn pytest_log() -> String {
    let mut lines = vec![
        "=== test session starts ===".to_owned(),
        "platform linux -- Python 3.12.4, pytest-8.3.2".to_owned(),
    ];
    lines.extend((0..40).map(|index| format!("tests/test_api.py::test_case_{index} PASSED")));
    for index in 0..12 {
        lines.push(format!("___________ test_case_fail_{index} ___________"));
        lines.push(format!(
            "E   AssertionError: expected status 200, got {}",
            500 + index
        ));
        lines.push(format!("ERROR tests/test_api.py::test_case_fail_{index}"));
        lines.push(String::new());
    }
    lines.extend((0..500).map(|_| {
        "WARNING  urllib3.connectionpool:connectionpool.py:869 - Connection pool is full, discarding connection".to_owned()
    }));
    lines.extend(
        (0..100)
            .map(|index| format!("INFO  app.api:api.py:{index} - request handled in {index}ms")),
    );
    lines.push(String::new());
    lines.push("=== short test summary info ===".to_owned());
    lines.push(
        "FAILED tests/test_api.py::test_case_fail_0 - AssertionError: expected status 200, got 500"
            .to_owned(),
    );
    lines.push("======== 12 failed, 40 passed in 3.42s ========".to_owned());
    lines.join("\n")
}

#[test]
fn every_error_survives_warnings_are_deduplicated_and_noise_dropped() {
    let source = pytest_log();
    let out = CrushStrategy::LogSelect.run(&source).expect("crushed");
    assert!(out.len() < source.len() / 2);
    let errors = out
        .lines()
        .filter(|line| line.starts_with("ERROR tests/test_api.py::"))
        .count();
    assert_eq!(errors, 12);
    let warnings = out
        .lines()
        .filter(|line| line.contains("Connection pool is full"))
        .count();
    assert!((1..=15).contains(&warnings), "{warnings}");
    let infos = out
        .lines()
        .filter(|line| line.contains("request handled in"))
        .count();
    assert!(infos <= 15, "{infos}");
    assert!(out.contains("lines omitted:"));
    assert!(out.contains("=== short test summary info ==="));
    assert_eq!(
        Some(out),
        CrushStrategy::LogSelect.run(&source),
        "deterministic"
    );
}

#[test]
fn short_logs_and_prose_pass_through() {
    let mut short = vec!["INFO start", "ERROR boom", "ERROR bang"];
    short.extend(["INFO tick"; 30]);
    assert_eq!(CrushStrategy::LogSelect.run(&short.join("\n")), None);
    let prose: Vec<String> = (0..60)
        .map(|index| {
            format!("Sentence number {index} talks about ordinary topics without any structure.")
        })
        .collect();
    assert_eq!(CrushStrategy::LogSelect.run(&prose.join("\n")), None);
    assert_eq!(crush(&prose.join("\n")), None);
}

#[test]
fn long_runtime_traces_collapse_to_their_head() {
    let mut lines = vec!["ERROR unhandled exception in worker pool".to_owned()];
    lines.extend((0..25).map(|index| {
        format!(
            "    at Worker.task (/app/node_modules/pkg/dist/worker.js:{}:{index})",
            10 + index
        )
    }));
    lines.push(
        "    at process.processTicksAndRejections (node:internal/process/task_queues:95:5)"
            .to_owned(),
    );
    lines.extend((0..40).map(|_| "info  heartbeat ok".to_owned()));
    let out = CrushStrategy::LogSelect
        .run(&lines.join("\n"))
        .expect("crushed");
    assert!(out.contains("at Worker.task (/app/node_modules/pkg/dist/worker.js:10:0)"));
    assert!(out.contains("frames collapsed]"));
    assert!(!out.contains("worker.js:20:10"));
    assert!(!out.contains("worker.js:34:24"));
}

#[test]
fn a_log_dispatches_to_log_select() {
    assert_eq!(
        crush(&pytest_log()).unwrap().strategy,
        CrushStrategy::LogSelect
    );
}

#[test]
fn a_lossy_log_output_that_drops_an_error_is_refused() {
    let mut lines = Vec::new();
    for index in 0..25u8 {
        lines.push(format!(
            "ERROR db_{}: connection lost shard {index}",
            (b'a' + index % 26) as char
        ));
        lines.extend((0..8).map(|tick| format!("INFO heartbeat ok shard-{index} tick-{tick}")));
    }
    let source = lines.join("\n");
    assert!(CrushStrategy::LogSelect.run(&source).is_some());
    assert_eq!(
        crush(&source),
        None,
        "21 distinct errors cannot all survive"
    );
}
