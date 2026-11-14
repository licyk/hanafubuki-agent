// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use super::*;
use hanafubuki_agent_approval::{
    ApprovalContext, ApprovalDecisionSource, ApprovalService, SystemApprovalClock,
};
use hanafubuki_agent_contracts::{
    AgentGeneration, AgentRevision, AgentSessionId, AgentSessionMode, AgentTurnId,
    CancellationToken, NoopAgentToolProgressSink, ToolCallContext, TurnExtensions,
};
use hanafubuki_agent_process::executor::manager::ExecutorManager;
use hanafubuki_agent_web_search::WebSearchConfig;
use std::{sync::Arc, time::Duration};

fn invocation(arguments: serde_json::Value) -> ToolInvocation {
    ToolInvocation {
        context: ToolCallContext {
            session_id: AgentSessionId::new(1),
            turn_id: AgentTurnId::new(1),
            generation: AgentGeneration::new(1),
            mode: AgentSessionMode::Default,
            session_revision: AgentRevision::new(1),
            call_id: "web-call".into(),
            image_input: false,
            origin: Default::default(),
        },
        arguments,
        cancellation: CancellationToken::default(),
        progress: Arc::new(NoopAgentToolProgressSink),
        plan_leases: Vec::new(),
        turn: TurnExtensions::default(),
    }
}
#[tokio::test]
async fn local_helpers_require_approval_for_the_exact_tool_and_arguments() {
    let mut config = WebSearchConfig::default();
    config.search.local_helpers_enabled = true;
    let service = WebSearchService::new(config)
        .unwrap()
        .with_local_runner(Some(Arc::new(crate::local::OwnedWebRunner {
            executor: ExecutorManager::default(),
        })));
    let mut call = invocation(serde_json::json!({"query":"first"}));
    let PermissionRequirement::ReviewRequired(action) =
        permission(&service, "web_search", &call).await.unwrap()
    else {
        panic!("local helpers require approval");
    };
    assert!(verify(&service, "web_search", &call, None).is_err());
    let context = ApprovalContext::from_canonical_bytes("web-call".into(), b"scope").unwrap();
    let approved = ApprovalService::new(Arc::new(SystemApprovalClock))
        .authorize(
            action,
            &context,
            Duration::from_secs(60),
            ApprovalDecisionSource::FullAccess,
        )
        .unwrap();
    assert!(verify(&service, "web_search", &call, Some(&approved)).is_ok());
    assert!(verify(&service, "research", &call, Some(&approved)).is_err());
    call.arguments["query"] = serde_json::json!("changed");
    assert!(verify(&service, "web_search", &call, Some(&approved)).is_err());
    assert!(matches!(
        permission(&service, "weather", &call).await.unwrap(),
        PermissionRequirement::NotRequired
    ));
    call.cancellation.cancel();
    assert!(matches!(
        permission(&service, "web_search", &call).await,
        Err(AgentToolError::Cancelled)
    ));
}
