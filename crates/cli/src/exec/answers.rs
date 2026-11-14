//! How `exec` answers what a turn asks, the session's or a sub-agent's:
//! approvals by the `--approve` policy, the model's questions only by a
//! person at a terminal.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, IsTerminal, Write};

use hanafubuki_agent::{
    AgentUserInputAnswer, AgentUserInputRequest, ApprovalRequest, ApprovalUserDecision,
};

use super::output::Output;

/// `--approve`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ApprovePolicy {
    Deny,
    /// The person at the terminal decides; without one, denied.
    Ask,
    /// The named tools are allowed, the others denied.
    Allow(BTreeSet<String>),
}

impl ApprovePolicy {
    pub(super) fn parse(policy: &str) -> Result<Self, String> {
        match policy {
            "deny" => Ok(Self::Deny),
            "ask" => Ok(Self::Ask),
            _ => {
                let tools = policy
                    .strip_prefix("allow:")
                    .map(|tools| {
                        tools
                            .split(',')
                            .map(str::trim)
                            .filter(|tool| !tool.is_empty())
                            .map(str::to_string)
                            .collect::<BTreeSet<_>>()
                    })
                    .filter(|tools| !tools.is_empty())
                    .ok_or_else(|| {
                        format!(
                            "--approve {policy} is not a policy: write deny, ask or \
                             allow:<tool>[,<tool>...]"
                        )
                    })?;
                Ok(Self::Allow(tools))
            }
        }
    }
}

pub(super) struct Answers {
    policy: ApprovePolicy,
    /// The tool of each call the session's turns started, by call id: an
    /// approval names its call (`operation_id`).
    tools: HashMap<String, String>,
}

impl Answers {
    pub(super) fn new(policy: ApprovePolicy) -> Self {
        Self {
            policy,
            tools: HashMap::new(),
        }
    }

    pub(super) fn tool_started(&mut self, call_id: &str, name: &str) {
        self.tools.insert(call_id.to_string(), name.to_string());
    }

    /// The tool of the session's call `call_id`.
    pub(super) fn tool(&self, call_id: &str) -> Option<&str> {
        self.tools.get(call_id).map(String::as_str)
    }

    /// The answer to `request`, a call of `tool` (`unknown tool` when not
    /// known) that `asker` makes (a sub-agent, by its title; the session
    /// itself when none), noted on `output`.
    pub(super) fn approval(
        &self,
        request: &ApprovalRequest,
        tool: Option<&str>,
        asker: Option<&str>,
        output: &mut dyn Output,
    ) -> ApprovalUserDecision {
        let tool = tool.unwrap_or("unknown tool");
        let summary = &match asker {
            Some(asker) => format!("sub-agent {asker}: {tool}: {}", request.action.summary),
            None => format!("{tool}: {}", request.action.summary),
        };
        let decision = match &self.policy {
            ApprovePolicy::Deny => ApprovalUserDecision::Deny,
            ApprovePolicy::Allow(tools) if tools.contains(tool) => ApprovalUserDecision::AllowOnce,
            ApprovePolicy::Allow(_) => ApprovalUserDecision::Deny,
            ApprovePolicy::Ask if terminal() => {
                ask_approval(summary, request.offers_allow_for_session)
            }
            ApprovePolicy::Ask => ApprovalUserDecision::Deny,
        };
        let answer = match decision {
            ApprovalUserDecision::AllowOnce => "allowed",
            ApprovalUserDecision::AllowForSession => "allowed for the session",
            ApprovalUserDecision::Deny => "denied",
        };
        output.note(&format!("approval {answer}: {summary}"));
        decision
    }

    /// The answers of the person at the terminal to what `asker` (a
    /// sub-agent, by its title; the session itself when none) asks; `None`
    /// without one, or when they give none.
    pub(super) fn questions(
        &self,
        request: &AgentUserInputRequest,
        asker: Option<&str>,
        output: &mut dyn Output,
    ) -> Option<Vec<AgentUserInputAnswer>> {
        if !terminal() {
            return None;
        }
        let mut answers = Vec::new();
        for question in &request.questions {
            let asker = asker.map(|asker| format!("sub-agent {asker}: "));
            let header = &question.header;
            eprintln!(
                "\n{}{header}\n{}",
                asker.unwrap_or_default(),
                question.question
            );
            for (index, option) in question.options.iter().enumerate() {
                eprintln!("  {}. {} — {}", index + 1, option.label, option.description);
            }
            let choice = read_line(&format!("choose 1-{}: ", question.options.len()))?;
            let option = choice
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|choice| question.options.get(choice.checked_sub(1)?))?;
            answers.push(AgentUserInputAnswer {
                question_id: question.id.clone(),
                option_id: option.id.clone(),
            });
        }
        output.note("questions answered");
        Some(answers)
    }
}

/// Whether a person can be asked: both the input and the prompts' output
/// are a terminal.
fn terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn ask_approval(summary: &str, offers_session: bool) -> ApprovalUserDecision {
    let choices = if offers_session {
        "[y]es, [s]ession, [N]o"
    } else {
        "[y]es, [N]o"
    };
    let answer = read_line(&format!("\nallow {summary}? {choices} "));
    match answer.as_deref().map(str::trim) {
        Some("y" | "Y" | "yes") => ApprovalUserDecision::AllowOnce,
        Some("s" | "S" | "session") if offers_session => ApprovalUserDecision::AllowForSession,
        _ => ApprovalUserDecision::Deny,
    }
}

fn read_line(prompt: &str) -> Option<String> {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

#[cfg(test)]
mod tests;
