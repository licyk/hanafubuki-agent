//! The approval rule of the actions this plugin runs: a `git clone` is how
//! Skills and tools are brought in, so it can change the model's future
//! instructions and executable surface. A person always decides it, even
//! under `AutoReview` or `FullAccess`.

use hanafubuki_agent_approval::{ApprovalAction, PermissionActionDetails};
use hanafubuki_agent_contracts::{ApprovalRule, ApprovalRuleVerdict};

pub(crate) struct GitCloneRequiresUser;

impl ApprovalRule for GitCloneRequiresUser {
    fn id(&self) -> &str {
        "execution.git-clone"
    }

    fn evaluate(&self, action: &ApprovalAction) -> Option<ApprovalRuleVerdict> {
        let clones = matches!(
            action.details(),
            Some(PermissionActionDetails::Command { program, argv, .. })
                if (program.ends_with("/git") || program.ends_with("\\git.exe") || program == "git")
                    && argv.iter().any(|argument| argument == "clone")
        );
        clones.then_some(ApprovalRuleVerdict::RequireUser)
    }
}
