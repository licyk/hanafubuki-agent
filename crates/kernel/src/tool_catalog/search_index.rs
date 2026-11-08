use super::*;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;

const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const MAX_SEARCH_TEXT_BYTES: usize = 64 * 1024;
const MAX_CACHED_SEARCH_INDEXES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolSearchInfo {
    pub search_text: String,
    pub output: ToolSpec,
    pub source_name: String,
    pub source_description: Option<String>,
}

pub(super) fn build_search_info(tool: &AgentPlannedTool) -> Option<AgentToolSearchInfo> {
    if matches!(tool.binding, AgentPlannedToolBinding::PlanSearch) {
        return None;
    }
    let mut parts = vec![
        tool.provider_id.as_str().to_string(),
        tool.tool_id.as_str().to_string(),
    ];
    append_spec_search_text(&tool.provider_spec, &mut parts);
    if let Some(description) = tool.namespace_description.as_deref() {
        push_part(&mut parts, description);
    }
    let mut search_text = parts.join(" ");
    truncate_utf8(&mut search_text, MAX_SEARCH_TEXT_BYTES);
    let mut output = tool.provider_spec.clone();
    set_tool_spec_deferred(&mut output, true);
    Some(AgentToolSearchInfo {
        search_text,
        output,
        source_name: tool.provider_id.as_str().to_string(),
        source_description: tool.namespace_description.clone(),
    })
}

fn append_spec_search_text(spec: &ToolSpec, parts: &mut Vec<String>) {
    match spec {
        ToolSpec::Function(function) => append_function_search_text(function, parts),
        ToolSpec::Namespace(namespace) => {
            push_part(parts, namespace.name.local_name());
            push_part(parts, &namespace.name.flattened_alias());
            push_part(parts, &namespace.description);
            for function in &namespace.tools {
                append_function_search_text(function, parts);
            }
        }
        ToolSpec::Freeform(freeform) => {
            push_part(parts, freeform.name.local_name());
            push_part(parts, &freeform.name.local_name().replace(['_', '-'], " "));
            push_part(parts, &freeform.description);
            push_part(parts, &freeform.format.syntax);
        }
        ToolSpec::ToolSearch(search) => push_part(parts, &search.description),
        ToolSpec::HostedWebSearch(_) => push_part(parts, "web search"),
    }
}

fn append_function_search_text(function: &FunctionToolSpec, parts: &mut Vec<String>) {
    push_part(parts, function.name.local_name());
    push_part(parts, &function.name.flattened_alias());
    push_part(parts, &function.name.local_name().replace(['_', '-'], " "));
    push_part(parts, &function.description);
    append_schema_search_text(&function.input_schema, parts, 0);
}

fn append_schema_search_text(schema: &Value, parts: &mut Vec<String>, depth: usize) {
    if depth > hanafubuki_agent_tool_contracts::MAX_TOOL_SCHEMA_DEPTH {
        return;
    }
    if let Some(description) = schema.get("description").and_then(Value::as_str) {
        push_part(parts, description);
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        let mut properties = properties.iter().collect::<Vec<_>>();
        properties.sort_by(|left, right| left.0.cmp(right.0));
        for (name, child) in properties {
            push_part(parts, name);
            push_part(parts, &name.replace(['_', '-'], " "));
            append_schema_search_text(child, parts, depth + 1);
        }
    }
    if let Some(items) = schema.get("items") {
        append_schema_search_text(items, parts, depth + 1);
    }
    for keyword in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = schema.get(keyword).and_then(Value::as_array) {
            for variant in variants {
                append_schema_search_text(variant, parts, depth + 1);
            }
        }
    }
}

fn push_part(parts: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        parts.push(value.to_string());
    }
}

fn truncate_utf8(value: &mut String, maximum_bytes: usize) {
    if value.len() <= maximum_bytes {
        return;
    }
    let mut boundary = maximum_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[derive(Clone, Default)]
pub struct AgentToolSearchIndexCache {
    inner: Arc<Mutex<AgentToolSearchIndexCacheState>>,
}

#[derive(Default)]
struct AgentToolSearchIndexCacheState {
    indexes: HashMap<[u8; 32], Arc<AgentToolSearchIndex>>,
    insertion_order: VecDeque<[u8; 32]>,
}

impl AgentToolSearchIndexCache {
    pub(super) fn get_or_build(
        &self,
        tools: &[AgentPlannedTool],
    ) -> Result<Arc<AgentToolSearchIndex>, AgentToolCatalogError> {
        let entries = tools
            .iter()
            .filter(|tool| tool.exposure == ToolExposure::Deferred)
            .filter_map(|tool| {
                tool.search_info
                    .clone()
                    .map(|info| AgentToolSearchDocument {
                        selection: AgentToolSelection::from_planned_tool(tool),
                        model_name: search_model_name(tool),
                        description: tool.model_spec.description.clone(),
                        info,
                    })
            })
            .collect::<Vec<_>>();
        let canonical =
            serde_json::to_vec(&entries).map_err(|_| AgentToolCatalogError::InvalidSchema)?;
        let fingerprint: [u8; 32] = Sha256::digest(canonical).into();
        {
            let cached = self.inner.lock();
            if let Some(index) = cached.indexes.get(&fingerprint) {
                return Ok(index.clone());
            }
        }
        let index = Arc::new(AgentToolSearchIndex::new(entries));
        let mut cached = self.inner.lock();
        if let Some(current) = cached.indexes.get(&fingerprint) {
            return Ok(current.clone());
        }
        while cached.indexes.len() >= MAX_CACHED_SEARCH_INDEXES {
            let Some(oldest) = cached.insertion_order.pop_front() else {
                break;
            };
            cached.indexes.remove(&oldest);
        }
        cached.insertion_order.push_back(fingerprint);
        cached.indexes.insert(fingerprint, index.clone());
        Ok(index)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentToolSearchDocument {
    selection: AgentToolSelection,
    model_name: String,
    description: String,
    info: AgentToolSearchInfo,
}

#[derive(Debug)]
pub(super) struct AgentToolSearchIndex {
    documents: Vec<IndexedDocument>,
    document_frequencies: BTreeMap<String, usize>,
    average_document_length: f64,
}

#[derive(Debug)]
struct IndexedDocument {
    source: AgentToolSearchDocument,
    term_frequencies: HashMap<String, usize>,
    length: usize,
}

impl AgentToolSearchIndex {
    fn new(documents: Vec<AgentToolSearchDocument>) -> Self {
        let mut document_frequencies = BTreeMap::new();
        let mut total_length = 0usize;
        let documents = documents
            .into_iter()
            .map(|source| {
                let tokens = tokenize(&source.info.search_text);
                total_length = total_length.saturating_add(tokens.len());
                let mut term_frequencies = HashMap::new();
                for token in tokens {
                    *term_frequencies.entry(token).or_insert(0) += 1;
                }
                for term in term_frequencies.keys() {
                    *document_frequencies.entry(term.clone()).or_insert(0) += 1;
                }
                let length = term_frequencies.values().sum();
                IndexedDocument {
                    source,
                    term_frequencies,
                    length,
                }
            })
            .collect::<Vec<_>>();
        let average_document_length = if documents.is_empty() {
            0.0
        } else {
            total_length as f64 / documents.len() as f64
        };
        Self {
            documents,
            document_frequencies,
            average_document_length,
        }
    }

    pub(super) fn document_count(&self) -> usize {
        self.documents.len()
    }

    pub(super) fn search(&self, query: &str, limit: usize) -> Vec<AgentToolSearchMatch> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() || self.documents.is_empty() {
            return Vec::new();
        }
        let document_count = self.documents.len() as f64;
        let mut matches = self
            .documents
            .iter()
            .filter_map(|document| {
                let mut score = 0.0;
                for term in &query_terms {
                    let Some(term_frequency) = document.term_frequencies.get(term).copied() else {
                        continue;
                    };
                    let document_frequency = self
                        .document_frequencies
                        .get(term)
                        .copied()
                        .unwrap_or_default() as f64;
                    let inverse_document_frequency = (1.0
                        + (document_count - document_frequency + 0.5) / (document_frequency + 0.5))
                        .ln();
                    let normalized_length = if self.average_document_length == 0.0 {
                        1.0
                    } else {
                        document.length as f64 / self.average_document_length
                    };
                    let frequency = term_frequency as f64;
                    score += inverse_document_frequency * frequency * (BM25_K1 + 1.0)
                        / (frequency + BM25_K1 * (1.0 - BM25_B + BM25_B * normalized_length));
                }
                (score > 0.0).then(|| AgentToolSearchMatch {
                    selection: document.source.selection.clone(),
                    specification: document.source.info.output.clone(),
                    model_name: document.source.model_name.clone(),
                    description: document.source.description.clone(),
                    score_milli: (score * 1_000.0).min(u32::MAX as f64) as u32,
                })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .score_milli
                .cmp(&left.score_milli)
                .then_with(|| left.model_name.cmp(&right.model_name))
                .then_with(|| left.selection.cmp(&right.selection))
        });
        matches.truncate(limit.clamp(1, MAX_AGENT_TOOL_SEARCH_RESULTS));
        matches
    }
}

/// The name a search result tells the model to call: the wire name where it
/// is a top-level one (a protocol without native search calls it so), the
/// tool's own name where the provider's search hands over its namespace.
fn search_model_name(tool: &AgentPlannedTool) -> String {
    match super::provider_table::ProviderToolName::of_spec(&tool.provider_spec) {
        Some(super::provider_table::ProviderToolName {
            namespace: None,
            name,
        }) => name,
        _ => tool.model_spec.name.clone(),
    }
}

fn tokenize(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut ascii = String::new();
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            ascii.push(character);
            continue;
        }
        if !ascii.is_empty() {
            tokens.push(std::mem::take(&mut ascii));
        }
        if character.is_alphanumeric() {
            tokens.push(character.to_string());
        }
    }
    if !ascii.is_empty() {
        tokens.push(ascii);
    }
    tokens
}

pub(crate) fn coalesce_search_specs(specs: impl IntoIterator<Item = ToolSpec>) -> Vec<ToolSpec> {
    let mut ordinary = Vec::new();
    let mut namespaces = BTreeMap::<String, NamespaceToolSpec>::new();
    for spec in specs {
        match spec {
            ToolSpec::Namespace(namespace) => {
                let key = namespace.name.flattened_alias();
                let entry = namespaces.entry(key).or_insert_with(|| NamespaceToolSpec {
                    name: namespace.name.clone(),
                    description: namespace.description.clone(),
                    tools: Vec::new(),
                });
                for tool in namespace.tools {
                    if !entry
                        .tools
                        .iter()
                        .any(|existing| existing.name == tool.name)
                    {
                        entry.tools.push(tool);
                    }
                }
            }
            spec => ordinary.push(spec),
        }
    }
    for namespace in namespaces.values_mut() {
        namespace.tools.sort_by(|left, right| {
            left.name
                .flattened_alias()
                .cmp(&right.name.flattened_alias())
        });
    }
    ordinary.extend(namespaces.into_values().map(ToolSpec::Namespace));
    ordinary.sort_by_key(tool_spec_sort_key);
    ordinary
}
