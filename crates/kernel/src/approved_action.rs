//! The note recording that the user approved an action automatic permission
//! review denied. The kernel commits it as a `ContinuationContext` under its
//! own provider and hook ids, which no plugin hook registers, so the history
//! format is unchanged; the model reads it as trusted context, and the
//! permission reviewer reads it as the user's authorization of that action.

use crate::model::{
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    ConversationItem,
};

/// The provider id of the note: the kernel's, never a plugin's.
pub(crate) const APPROVED_ACTION_PROVIDER_ID: &str = "kernel";
/// The hook id of the note.
pub(crate) const APPROVED_ACTION_HOOK_ID: &str = "approved-denied-action";

const APPROVED_ACTION_GENERATION: AgentContinuationHookGeneration = {
    let mut bytes = [0; 32];
    bytes[31] = 1;
    AgentContinuationHookGeneration(bytes)
};

/// The note approving the action `action_summary` describes.
pub(crate) fn approved_action_note(action_summary: &str) -> ConversationItem {
    ConversationItem::ContinuationContext {
        provider_id: AgentContinuationHookProviderId::parse(APPROVED_ACTION_PROVIDER_ID.into())
            .expect("the approved action provider id is a valid id"),
        hook_id: AgentContinuationHookId::parse(APPROVED_ACTION_HOOK_ID.into())
            .expect("the approved action hook id is a valid id"),
        generation: APPROVED_ACTION_GENERATION,
        content: format!(
            "The user has manually approved a specific action that automatic permission review previously denied.\n\n\
             Treat this as approval to perform that exact action in the same context in which it was originally requested.\n\
             Do not assume this also authorizes similar operations with different payloads.\n\n\
             Approved action:\n{action_summary}"
        ),
    }
}

/// The text of `item` if it is an approved action note.
pub(crate) fn approved_action_note_text(item: &ConversationItem) -> Option<&str> {
    match item {
        ConversationItem::ContinuationContext {
            provider_id,
            hook_id,
            content,
            ..
        } if provider_id.as_str() == APPROVED_ACTION_PROVIDER_ID
            && hook_id.as_str() == APPROVED_ACTION_HOOK_ID =>
        {
            Some(content)
        }
        _ => None,
    }
}
