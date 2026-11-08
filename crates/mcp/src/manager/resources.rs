use super::catalog::MAX_RECORDED_CATALOG_ISSUES;
use super::model::McpLiveConnection;
use crate::{
    McpConnectionGeneration, McpDiagnosticOutcome, McpDiagnosticStage, McpDiagnosticStore,
    McpServerId,
};
use std::sync::Arc;
use std::time::Duration;

/// What resource discovery found for a new connection.
pub(super) struct McpResourceDiscovery {
    pub(super) resource_count: usize,
    pub(super) template_count: usize,
    /// Listed entries that did not decode, by name (`listing::McpListing`).
    pub(super) undecodable_resources: Vec<String>,
    pub(super) undecodable_templates: Vec<String>,
    /// Why a listing failed as a whole, when one did.
    pub(super) failure: Option<String>,
}

impl McpResourceDiscovery {
    /// The `resource_discovery_partial` message, or `None` when discovery
    /// used everything it listed.
    pub(super) fn partial_summary(&self) -> Option<String> {
        let skipped = (
            self.undecodable_resources.len(),
            self.undecodable_templates.len(),
        );
        if self.failure.is_none() && skipped == (0, 0) {
            return None;
        }
        let mut summary = format!(
            "discovered {} resources and {} templates",
            self.resource_count, self.template_count
        );
        if skipped != (0, 0) {
            summary.push_str(&format!(
                "; skipped {} resources and {} templates that could not be decoded",
                skipped.0, skipped.1
            ));
        }
        if let Some(failure) = &self.failure {
            summary.push_str(&format!("; {failure}"));
        }
        Some(summary)
    }
}

pub(super) async fn discover_resources(
    connection: &Arc<McpLiveConnection>,
    timeout: Duration,
) -> McpResourceDiscovery {
    if !connection.supports_resources() {
        return McpResourceDiscovery {
            resource_count: 0,
            template_count: 0,
            undecodable_resources: Vec::new(),
            undecodable_templates: Vec::new(),
            failure: None,
        };
    }
    let (resources, templates) = tokio::join!(
        connection.list_resources(timeout),
        connection.list_resource_templates(timeout)
    );
    let failure = match (resources.as_ref().err(), templates.as_ref().err()) {
        (None, None) => None,
        (Some(error), None) => Some(format!("resource discovery failed: {error}")),
        (None, Some(error)) => Some(format!("resource template discovery failed: {error}")),
        (Some(resource), Some(template)) => Some(format!(
            "resource and template discovery failed: {resource}; {template}"
        )),
    };
    let (resource_count, undecodable_resources) = resources
        .map(|listing| (listing.items.len(), listing.undecodable))
        .unwrap_or_default();
    let (template_count, undecodable_templates) = templates
        .map(|listing| (listing.items.len(), listing.undecodable))
        .unwrap_or_default();
    McpResourceDiscovery {
        resource_count,
        template_count,
        undecodable_resources,
        undecodable_templates,
        failure,
    }
}

/// A listed resource entry a diagnostic names.
#[derive(Clone, Copy)]
pub(super) enum McpResourceEntryKind {
    Resource,
    Template,
}

/// The server and connection a resource diagnostic belongs to.
pub(super) struct McpResourceDiagnosticScope<'a> {
    pub(super) diagnostics: &'a McpDiagnosticStore,
    pub(super) server_id: &'a McpServerId,
    pub(super) transport: &'a str,
    pub(super) config_generation: [u8; 32],
    pub(super) runtime_generation: Option<McpConnectionGeneration>,
}

impl McpResourceEntryKind {
    /// The diagnostic code, the per-entry message and the plural noun.
    fn words(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Resource => (
                "resource_skipped_undecodable",
                "MCP resource was skipped: it could not be decoded",
                "resources",
            ),
            Self::Template => (
                "resource_template_skipped_undecodable",
                "MCP resource template was skipped: it could not be decoded",
                "resource templates",
            ),
        }
    }
}

impl McpResourceDiagnosticScope<'_> {
    /// Records each listed entry discovery could not decode as a Warning
    /// naming only the entry, up to [`MAX_RECORDED_CATALOG_ISSUES`] per
    /// listing like the tool catalog's issues.
    pub(super) fn record_undecodable(&self, kind: McpResourceEntryKind, names: &[String]) {
        let (code, message, noun) = kind.words();
        if names.is_empty() {
            return;
        }
        log::warn!(
            "MCP server id={} skipped {} undecodable {noun}",
            self.server_id.as_str(),
            names.len()
        );
        for name in names.iter().take(MAX_RECORDED_CATALOG_ISSUES) {
            self.record(code, message, Some(name.clone()));
        }
    }

    /// Records one Warning for an Agent's page that left entries out: the
    /// Agent may page again and again, and each page must cost the server's
    /// other diagnostics at most one entry.
    pub(super) fn record_page_skipped(&self, kind: McpResourceEntryKind, skipped: usize) {
        let (code, _, noun) = kind.words();
        if skipped == 0 {
            return;
        }
        self.record(
            code,
            &format!("an MCP {noun} page left out {skipped} entries that could not be decoded"),
            None,
        );
    }

    fn record(&self, code: &str, message: &str, entry: Option<String>) {
        self.diagnostics.record(
            self.server_id.clone(),
            McpDiagnosticStage::DiscoverResources,
            McpDiagnosticOutcome::Warning,
            code,
            message,
            self.transport,
            self.config_generation,
            self.runtime_generation,
            entry,
            false,
            "none",
        );
    }
}
