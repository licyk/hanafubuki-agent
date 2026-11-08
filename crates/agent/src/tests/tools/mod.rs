use super::*;
use crate::test_support::TestPaths;
use crate::AgentConfig;

fn reasoning_assistant_events(text: &str) -> Vec<u8> {
    format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-reasoning\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"reasoning\",\"id\":\"reason-1\",\"summary\":[]}}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"reasoning\",\"id\":\"reason-1\",\"summary\":[{{\"type\":\"summary_text\",\"text\":\"private fixture reasoning\"}}]}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"message\",\"id\":\"message\",\"phase\":\"final_answer\"}}}}\n\n",
            "data: {{\"type\":\"response.output_text.delta\",\"item_id\":\"message\",\"delta\":{text:?}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"message\",\"id\":\"message\",\"phase\":\"final_answer\",\"content\":[{{\"type\":\"output_text\",\"text\":{text:?}}}]}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-reasoning\",\"end_turn\":true}}}}\n\n"
        ),
        text = text,
    )
    .into_bytes()
}

/// Build a service after running `configure` against its paths and config, so a
/// test can decide exactly which provider/credential state exists (including
/// none). Factored so the missing-provider and missing-credential contracts
/// share the construction with the fully configured case.
pub(super) fn service_with(
    name: &str,
    configure: impl FnOnce(&TestPaths, &AgentConfig),
) -> (AgentApplicationService, TestPaths) {
    let (service, paths, _) = service_with_config(name, configure);
    (service, paths)
}

pub(super) fn service_with_config(
    name: &str,
    configure: impl FnOnce(&TestPaths, &AgentConfig),
) -> (AgentApplicationService, TestPaths, AgentConfig) {
    service_with_host(name, test_host, configure)
}

pub(super) fn configured_service(
    name: &str,
    base_url: &str,
) -> (AgentApplicationService, TestPaths) {
    service_with(name, |_, config| {
        configure_test_provider(config, base_url, None);
    })
}

mod execution;
mod filesystem;
mod mcp;
mod skills;
