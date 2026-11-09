// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the json-fold strategy of `kernel/src/crush.ts` of acp-kernel 0.0.189 at
// commit `4be21b08e` (its concept adapted from headroom's SmartCrusher,
// headroomlabs-ai/headroom, Apache-2.0). Copyright (c) 2026 ranxianglei. MIT
// License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: canonical forms are recomputed instead of
// cached by object identity; an object keeps its keys in document order,
// where JavaScript would move integer-like keys first.

use crate::json::{quote, Json};
use crate::text::utf16_len;

const MAX_DEPTH: usize = 12;
const MAX_FOLDS: usize = 1000;
const RUN_MIN: usize = 4;
const HOIST_MIN: usize = 4;
const CRUSH_KEY: &str = "__acp_crush";

/// Folds a JSON document losslessly: an array of objects whose fields are
/// constant becomes a `rows` envelope (`const` merged into each of `items`),
/// a run of four or more identical elements an `identical-run` envelope.
/// `None` when the text is not JSON, nothing folds or nothing is saved; a
/// document that has a `__acp_crush` key of its own cannot be told apart
/// from a folded one.
pub fn crush_json(text: &str) -> Option<String> {
    let parsed = Json::parse(text)?;
    let mut folds = 0;
    let out = compact_value(&parsed, 0, &mut folds);
    if folds == 0 {
        return None;
    }
    let out = out.to_js_text();
    (utf16_len(&out) < utf16_len(text)).then_some(out)
}

fn compact_value(value: &Json, depth: usize, folds: &mut usize) -> Json {
    if depth > MAX_DEPTH || *folds >= MAX_FOLDS {
        return value.clone();
    }
    match value {
        Json::Array(items) => compact_array(items, depth, folds),
        Json::Object(entries) => Json::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), compact_value(value, depth + 1, folds)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The shortest of the array as it is (its elements compacted), its runs
/// folded and its constant fields hoisted.
fn compact_array(items: &[Json], depth: usize, folds: &mut usize) -> Json {
    if items.len() < RUN_MIN && items.len() < HOIST_MIN {
        return Json::Array(
            items
                .iter()
                .map(|item| compact_value(item, depth + 1, folds))
                .collect(),
        );
    }
    if *folds >= MAX_FOLDS {
        return Json::Array(items.to_vec());
    }
    let mut plain_folds = 0;
    let plain = Json::Array(
        items
            .iter()
            .map(|item| compact_value(item, depth + 1, &mut plain_folds))
            .collect(),
    );
    let mut winner = Candidate {
        len: js_len(&plain),
        value: plain,
        folds: plain_folds,
    };
    let candidates = [
        (items.len() >= RUN_MIN)
            .then(|| build_run_fold(items, depth))
            .flatten(),
        (items.len() >= HOIST_MIN && items.iter().all(|item| matches!(item, Json::Object(_))))
            .then(|| build_const_hoist(items, depth))
            .flatten(),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.len < winner.len {
            winner = candidate;
        }
    }
    *folds += winner.folds;
    winner.value
}

struct Candidate {
    value: Json,
    len: usize,
    folds: usize,
}

fn build_run_fold(items: &[Json], depth: usize) -> Option<Candidate> {
    let canon: Vec<String> = items.iter().map(canon_of).collect();
    let mut segments = Vec::new();
    let mut runs = 0;
    let mut inner = 0;
    let mut start = 0;
    while start < items.len() {
        let mut end = start;
        while end + 1 < items.len() && canon[end + 1] == canon[start] {
            end += 1;
        }
        let length = end - start + 1;
        if length >= RUN_MIN {
            runs += 1;
            segments.push(Json::Object(vec![
                (
                    CRUSH_KEY.to_owned(),
                    Json::String("identical-run".to_owned()),
                ),
                ("count".to_owned(), Json::Number(length.into())),
                (
                    "item".to_owned(),
                    compact_value(&items[start], depth + 1, &mut inner),
                ),
            ]));
        } else {
            segments.extend(
                items[start..=end]
                    .iter()
                    .map(|item| compact_value(item, depth + 1, &mut inner)),
            );
        }
        start = end + 1;
    }
    if runs == 0 {
        return None;
    }
    let value = Json::Array(segments);
    Some(Candidate {
        len: js_len(&value),
        value,
        folds: runs + inner,
    })
}

fn build_const_hoist(items: &[Json], depth: usize) -> Option<Candidate> {
    let rows: Vec<&[(String, Json)]> = items
        .iter()
        .filter_map(|item| match item {
            Json::Object(entries) => Some(entries.as_slice()),
            _ => None,
        })
        .collect();
    let first = *rows.first()?;
    let mut keys: Vec<&str> = Vec::new();
    for row in &rows {
        for (key, _) in row.iter() {
            if !keys.contains(&key.as_str()) {
                keys.push(key);
            }
        }
    }
    // A key missing from the first row is never hoisted: absent and null
    // would compare equal, and the field would vanish from every row.
    let constant: Vec<(String, Json)> = keys
        .iter()
        .filter_map(|key| {
            let value = field(first, key)?;
            let canon = canon_of(value);
            rows[1..]
                .iter()
                .all(|row| field(row, key).is_some_and(|other| canon_of(other) == canon))
                .then(|| ((*key).to_owned(), value.clone()))
        })
        .collect();
    if constant.is_empty() {
        return None;
    }
    let is_constant = |key: &str| constant.iter().any(|(name, _)| name == key);
    let varying: Vec<Json> = rows
        .iter()
        .map(|row| {
            Json::Object(
                row.iter()
                    .filter(|(key, _)| !is_constant(key))
                    .cloned()
                    .collect(),
            )
        })
        .collect();
    let has_varying = varying
        .iter()
        .any(|row| matches!(row, Json::Object(entries) if !entries.is_empty()));
    let mut inner = 0;
    let mut envelope = vec![
        (CRUSH_KEY.to_owned(), Json::String("rows".to_owned())),
        ("rows".to_owned(), Json::Number(rows.len().into())),
        (
            "const".to_owned(),
            compact_value(&Json::Object(constant.clone()), depth + 1, &mut inner),
        ),
    ];
    if has_varying {
        let items = varying
            .iter()
            .map(|row| compact_value(row, depth + 1, &mut inner))
            .collect();
        envelope.push(("items".to_owned(), Json::Array(items)));
    }
    let value = Json::Object(envelope);
    let len = js_len(&value);
    if len >= js_len(&Json::Array(items.to_vec())) {
        return None;
    }
    Some(Candidate {
        value,
        len,
        folds: 1 + inner,
    })
}

fn field<'a>(row: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    row.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// A form equal for equal values whatever their key order.
fn canon_of(value: &Json) -> String {
    match value {
        Json::Array(items) => format!(
            "[{}]",
            items.iter().map(canon_of).collect::<Vec<_>>().join(",")
        ),
        Json::Object(entries) => {
            let mut entries: Vec<String> = entries
                .iter()
                .map(|(key, value)| format!("{}:{}", quote(key), canon_of(value)))
                .collect();
            entries.sort();
            format!("{{{}}}", entries.join(","))
        }
        other => other.to_js_text(),
    }
}

fn js_len(value: &Json) -> usize {
    utf16_len(&value.to_js_text())
}
