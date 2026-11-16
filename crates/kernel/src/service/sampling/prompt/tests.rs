use super::*;

fn fragment(content: &str) -> ModelInputItem {
    ModelInputItem::ContextFragment {
        kind: "catalog".to_string(),
        role: ModelContextRole::User,
        content: content.to_string(),
    }
}

#[test]
fn snapshots_merge_same_kind_and_append_only_changes_and_withdrawals() {
    let initial = vec![fragment("first"), fragment("second")];
    let mut history = RequestContext::snapshot_updates(std::iter::empty(), initial.clone());
    assert_eq!(history.len(), 1);
    assert!(matches!(&history[0], ConversationItem::PromptContext {
        content: Some(content), role: ModelContextRole::User, ..
    } if content == "first\n\nsecond"));
    assert!(RequestContext::snapshot_updates(history.iter(), initial).is_empty());

    let old = history.clone();
    let changed = RequestContext::snapshot_updates(history.iter(), vec![fragment("new")]);
    history.extend(changed);
    assert_eq!(&history[..old.len()], &old);
    assert!(RequestContext::snapshot_updates(history.iter(), vec![fragment("new")]).is_empty());

    let removed = RequestContext::snapshot_updates(history.iter(), Vec::new());
    assert_eq!(removed.len(), 1);
    assert!(matches!(
        &removed[0],
        ConversationItem::PromptContext {
            content: None,
            role: ModelContextRole::User,
            ..
        }
    ));
    assert!(
        matches!(removed[0].as_model_input(), Some(ModelInputItem::ContextFragment {
        content, ..
    }) if content.contains("no longer active"))
    );
    history.extend(removed);
    assert!(RequestContext::snapshot_updates(history.iter(), Vec::new()).is_empty());

    let reopened: Vec<ConversationItem> =
        serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
    assert_eq!(reopened, history);
    assert!(RequestContext::snapshot_updates(reopened.iter(), Vec::new()).is_empty());
    assert_eq!(
        RequestContext::snapshot_updates(reopened.iter(), vec![fragment("new")]).len(),
        1
    );
}
