//! The wire names of one provider request: every planned tool's structured
//! name fitted to what the protocol takes, and the lookup a returned call
//! resolves through. Nothing splits or joins a returned name.

use std::collections::BTreeMap;

use hanafubuki_agent_tool_contracts::ToolProviderCapabilities;

use super::contributor::push_optional_omission;
use super::*;

/// Hex digits of the identity hash a fitted name ends with, after a `_`.
const SUFFIX_HEX_DIGITS: usize = 12;

/// A tool's name on the wire: a native namespace and the name in it, or a
/// top-level name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ProviderToolName {
    pub(super) namespace: Option<String>,
    pub(super) name: String,
}

impl ProviderToolName {
    /// The wire name a planned tool's provider spec carries once
    /// [`assign_provider_names`] wrote it; `None` for a spec without a name.
    pub(super) fn of_spec(spec: &ToolSpec) -> Option<Self> {
        match spec {
            ToolSpec::Function(function) => Some(Self {
                namespace: None,
                name: function.name.local_name().to_string(),
            }),
            ToolSpec::Freeform(freeform) => Some(Self {
                namespace: None,
                name: freeform.name.local_name().to_string(),
            }),
            ToolSpec::Namespace(namespace) => namespace.tools.first().map(|function| Self {
                namespace: Some(namespace.name.local_name().to_string()),
                name: function.name.local_name().to_string(),
            }),
            ToolSpec::ToolSearch(_) | ToolSpec::HostedWebSearch(_) => None,
        }
    }

    /// The flattened form names are unique by, whichever form the wire uses.
    fn flattened(&self) -> String {
        match &self.namespace {
            Some(namespace) => format!("{namespace}__{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// A plan's wire names, each to the index of the registration it names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ProviderToolTable {
    names: BTreeMap<ProviderToolName, usize>,
}

impl ProviderToolTable {
    pub(super) fn index(registrations: &[AgentPlannedTool]) -> Self {
        let mut names = BTreeMap::new();
        for (index, registration) in registrations.iter().enumerate() {
            if let Some(name) = ProviderToolName::of_spec(&registration.provider_spec) {
                names.entry(name).or_insert(index);
            }
        }
        Self { names }
    }

    /// The registration a returned call names; `None` for a name the
    /// request did not carry.
    pub(super) fn get(&self, namespace: Option<&str>, name: &str) -> Option<usize> {
        self.names
            .get(&ProviderToolName {
                namespace: namespace.map(str::to_string),
                name: name.to_string(),
            })
            .copied()
    }
}

/// Writes every planned tool's wire name into its provider spec: its native
/// namespace and local name where the protocol takes namespaces, its
/// flattened name otherwise, fitted to the protocol's name limit. Names are
/// unique over their flattened form, and `tool_search` is kept for the
/// plan's search. In planning order (trust, then name, provider and tool id)
/// the first tool keeps its name; one that collides with an earlier one, or
/// is too long, ends with a hash of its identity instead, so it keeps its
/// name from step to step. A collision is reported once in `omissions`
/// (the tool stays planned under its new name); a fitted long name is not.
pub(super) fn assign_provider_names(
    tools: &mut [AgentPlannedTool],
    capabilities: ToolProviderCapabilities,
    omissions: &mut Vec<AgentToolOmission>,
) -> Result<(), AgentToolCatalogError> {
    let mut order = (0..tools.len()).collect::<Vec<_>>();
    order.sort_by(|&left, &right| planning_order(&tools[left], &tools[right]));
    let mut taken =
        HashSet::from([hanafubuki_agent_model_api::FALLBACK_TOOL_SEARCH_NAME.to_string()]);
    let mut namespace_owners = BTreeMap::<String, AgentToolProviderId>::new();
    for index in order {
        let tool = &mut tools[index];
        let Some(name) = tool.provider_spec.name().cloned() else {
            continue;
        };
        let namespace = native_namespace(tool, &name, capabilities, &mut namespace_owners);
        let preferred = ProviderToolName {
            name: match namespace {
                Some(_) => name.local_name().to_string(),
                None => name.flattened_alias(),
            },
            namespace,
        };
        let flattened = preferred.flattened();
        let collides = taken.contains(&flattened);
        let wire = if !collides && flattened.len() <= capabilities.maximum_tool_name_bytes {
            taken.insert(flattened);
            preferred
        } else {
            fitted_name(
                tool,
                preferred,
                capabilities.maximum_tool_name_bytes,
                &mut taken,
            )
        };
        if collides {
            push_optional_omission(
                omissions,
                tool.provider_id.as_str(),
                Some(tool.model_spec.name.clone()),
                format!(
                    "renamed to `{}`: an earlier tool has its name",
                    wire.flattened()
                ),
            );
        }
        tool.provider_spec = wire_spec(
            tool.provider_spec.clone(),
            wire,
            tool.namespace_description.as_deref(),
        )?;
    }
    Ok(())
}

fn planning_order(left: &AgentPlannedTool, right: &AgentPlannedTool) -> std::cmp::Ordering {
    fn trust_rank(trust: AgentToolTrust) -> u8 {
        match trust {
            AgentToolTrust::Trusted => 0,
            AgentToolTrust::RequiredExternal => 1,
            AgentToolTrust::OptionalExternal => 2,
        }
    }
    trust_rank(left.trust)
        .cmp(&trust_rank(right.trust))
        .then_with(|| left.model_spec.name.cmp(&right.model_spec.name))
        .then_with(|| left.provider_id.cmp(&right.provider_id))
        .then_with(|| left.tool_id.cmp(&right.tool_id))
}

/// The namespace a function keeps on a protocol that takes namespaces. A
/// namespace belongs to the first provider that uses it, so two providers'
/// tools never merge into one; another provider's tool, or a namespace too
/// long to be a name, goes on the wire flattened instead.
fn native_namespace(
    tool: &AgentPlannedTool,
    name: &ToolName,
    capabilities: ToolProviderCapabilities,
    owners: &mut BTreeMap<String, AgentToolProviderId>,
) -> Option<String> {
    if !capabilities.namespace_tools
        || name.namespace().is_empty()
        || !matches!(tool.provider_spec, ToolSpec::Function(_))
    {
        return None;
    }
    let namespace = name.namespace().join("__");
    ToolName::parse(namespace.clone()).ok()?;
    let owner = owners
        .entry(namespace.clone())
        .or_insert_with(|| tool.provider_id.clone());
    (*owner == tool.provider_id).then_some(namespace)
}

/// `preferred` cut to fit `limit` with `_` and a hash of the tool's
/// identity appended, kept in its namespace while the namespace leaves room;
/// the hash takes an attempt counter until the flattened name is free.
fn fitted_name(
    tool: &AgentPlannedTool,
    preferred: ProviderToolName,
    limit: usize,
    taken: &mut HashSet<String>,
) -> ProviderToolName {
    let flattened = preferred.flattened();
    let mut attempt = 0u32;
    loop {
        let suffix = identity_suffix(tool, attempt);
        let candidate = match &preferred.namespace {
            Some(namespace) if namespace.len() + 2 + suffix.len() < limit => {
                let room = limit - namespace.len() - 2 - suffix.len();
                ProviderToolName {
                    namespace: Some(namespace.clone()),
                    name: format!("{}{suffix}", ascii_prefix(&preferred.name, room)),
                }
            }
            _ => ProviderToolName {
                namespace: None,
                name: format!(
                    "{}{suffix}",
                    ascii_prefix(&flattened, limit.saturating_sub(suffix.len()))
                ),
            },
        };
        if taken.insert(candidate.flattened()) {
            return candidate;
        }
        attempt = attempt.wrapping_add(1);
    }
}

fn identity_suffix(tool: &AgentPlannedTool, attempt: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tool.provider_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(tool.tool_id.as_str().as_bytes());
    if attempt > 0 {
        hasher.update(attempt.to_le_bytes());
    }
    let digest: [u8; 32] = hasher.finalize().into();
    let mut suffix = String::from("_");
    for byte in &digest[..SUFFIX_HEX_DIGITS / 2] {
        suffix.push_str(&format!("{byte:02x}"));
    }
    suffix
}

/// Tool names are ASCII ([`ToolName`] validates them), so a byte prefix is
/// a character prefix.
fn ascii_prefix(value: &str, bytes: usize) -> &str {
    &value[..value.len().min(bytes)]
}

fn wire_spec(
    spec: ToolSpec,
    wire: ProviderToolName,
    namespace_description: Option<&str>,
) -> Result<ToolSpec, AgentToolCatalogError> {
    let name = ToolName::parse(wire.name).map_err(|_| AgentToolCatalogError::InvalidToolName)?;
    Ok(match (spec, wire.namespace) {
        (ToolSpec::Function(function), Some(namespace)) => ToolSpec::Namespace(NamespaceToolSpec {
            description: namespace_description
                .map(str::to_string)
                .unwrap_or_else(|| format!("Tools provided by {namespace}.")),
            name: ToolName::parse(namespace).map_err(|_| AgentToolCatalogError::InvalidToolName)?,
            tools: vec![FunctionToolSpec { name, ..function }],
        }),
        (ToolSpec::Function(function), None) => {
            ToolSpec::Function(FunctionToolSpec { name, ..function })
        }
        (ToolSpec::Freeform(freeform), _) => {
            ToolSpec::Freeform(hanafubuki_agent_tool_contracts::FreeformToolSpec {
                name,
                ..freeform
            })
        }
        (other, _) => other,
    })
}
