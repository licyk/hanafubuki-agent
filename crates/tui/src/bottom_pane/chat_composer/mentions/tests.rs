// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;
use crate::app_event::AppEventSender;
use crate::bottom_pane::skill_popup::MentionItem;
use crate::file_search::FileMatch;
use crate::mentions::Catalog;
use crate::user_message::UserMessage;
use std::sync::mpsc::{channel, Receiver};

fn item(name: &str, id: &str) -> MentionItem {
    MentionItem {
        display_name: name.into(),
        description: Some("Useful capability".into()),
        insert_text: format!("${name}"),
        search_terms: vec![name.into()],
        path: Some(format!("skill://{id}")),
        category_tag: Some("[Skill]".into()),
        sort_rank: 1,
    }
}

fn composer() -> (ChatComposer, Receiver<AppEvent>) {
    let (tx, rx) = channel();
    let mut composer =
        ChatComposer::new(true, AppEventSender::new(tx), false, "Ask anything".into());
    let skills = vec![item("shared", "system:one"), item("shared", "system:two")];
    let at = vec![
        Candidate {
            display_name: "Sample Plugin".into(),
            description: Some("Plugin description".into()),
            search_terms: vec!["sample".into()],
            mention_type: MentionType::Plugin,
            selection: Selection::Tool {
                insert_text: "@Sample".into(),
                path: Some("plugin://sample@local".into()),
            },
        },
        Candidate {
            display_name: "shared".into(),
            description: None,
            search_terms: vec!["shared".into()],
            mention_type: MentionType::Skill,
            selection: Selection::Tool {
                insert_text: "$shared".into(),
                path: Some("skill://system:one".into()),
            },
        },
    ];
    composer.set_mention_catalog(Catalog { at, dollar: skills });
    (composer, rx)
}
fn key(c: &mut ChatComposer, code: KeyCode) -> InputResult {
    c.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE))
        .0
}

#[test]
fn selected_duplicate_skill_keeps_identity_through_edit_recall_and_submission() {
    let (mut c, rx) = composer();
    c.set_text_content("use $sha".into());
    key(&mut c, KeyCode::Down);
    key(&mut c, KeyCode::Enter);
    assert_eq!(c.textarea.text(), "use $shared ");
    assert_eq!(
        c.current_text_with_pending(),
        "use [$shared](skill://system:two) "
    );
    assert!(rx
        .try_iter()
        .any(|event| matches!(event,AppEvent::RecordMention(path) if path=="skill://system:two")));
    c.apply_external_edit(format!("prefix {}", c.current_text_with_pending()));
    assert_eq!(c.textarea.text(), "prefix use $shared");
    assert_eq!(
        c.current_text_with_pending(),
        "prefix use [$shared](skill://system:two)"
    );
    let saved: UserMessage = c.current_text_with_pending().as_str().into();
    c.set_message(saved.clone());
    assert_eq!(key(&mut c, KeyCode::Enter), InputResult::Submitted(saved));
}

#[test]
fn backspace_removes_an_entire_bound_mention() {
    let (mut c, _) = composer();
    c.set_message("before [$shared](skill://system:two)".into());
    key(&mut c, KeyCode::Backspace);
    assert_eq!(c.textarea.text(), "before ");
    assert_eq!(c.current_text_with_pending(), "before ");
}

#[test]
fn unified_tabs_and_control_navigation_match_codex() {
    let (mut c, _) = composer();
    c.set_text_content("@sha".into());
    key(&mut c, KeyCode::Right);
    let ActivePopup::Mention(p) = &c.active_popup else {
        panic!("picker")
    };
    assert!(p.selected().is_none());
    key(&mut c, KeyCode::Right);
    key(&mut c, KeyCode::Right);
    assert_eq!(key(&mut c, KeyCode::Tab), InputResult::None);
    assert_eq!(
        c.current_text_with_pending(),
        "[$shared](skill://system:one) "
    );
    c.set_text_content("$sha".into());
    c.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    key(&mut c, KeyCode::Tab);
    assert_eq!(
        c.current_text_with_pending(),
        "[$shared](skill://system:two) "
    );
}

#[test]
fn shell_tokens_do_not_open_skill_picker() {
    let (mut c, _) = composer();
    for text in [
        "$HOME",
        "$1",
        "$-",
        "${shared}",
        "$(pwd)",
        "email@example.com",
    ] {
        c.set_text_content(text.into());
        assert!(matches!(c.active_popup, ActivePopup::None), "{text}");
    }
    c.set_text_content("$".into());
    assert!(matches!(c.active_popup, ActivePopup::Skill(_)));
}

#[test]
fn escape_dismissal_is_occurrence_scoped_and_unmatched_at_can_submit() {
    let (mut c, _) = composer();
    c.set_text_content("$sha $sha".into());
    key(&mut c, KeyCode::Esc);
    c.sync_popups();
    assert!(matches!(c.active_popup, ActivePopup::None));
    c.textarea.set_cursor(3);
    c.sync_popups();
    assert!(matches!(c.active_popup, ActivePopup::Skill(_)));
    c.set_text_content("@missing".into());
    assert_eq!(
        key(&mut c, KeyCode::Enter),
        InputResult::Submitted("@missing".into())
    );
}

#[test]
fn asynchronous_tasks_do_not_replace_a_newer_query_or_disappear_after_catalog_load() {
    let (mut c, _) = composer();
    c.set_text_content("@task".into());
    let task = Candidate {
        display_name: "task 界 [debug]".into(),
        description: None,
        search_terms: vec!["task".into()],
        mention_type: MentionType::Task,
        selection: Selection::Tool {
            insert_text: "@task 界 [debug]".into(),
            path: Some("thread://52".into()),
        },
    };
    c.set_mention_tasks("old".into(), vec![task.clone()]);
    assert!(!c
        .mention_catalog
        .at
        .iter()
        .any(|x| x.mention_type == MentionType::Task));
    c.set_mention_tasks("task".into(), vec![task]);
    c.set_mention_catalog(Catalog::default());
    key(&mut c, KeyCode::Enter);
    assert_eq!(c.textarea.text(), "@task 界 [debug] ");
    let saved = c.current_text_with_pending();
    c.set_message(saved.as_str().into());
    assert_eq!(c.current_text_with_pending(), saved);
}

#[test]
fn file_completion_preserves_paste_and_bound_mentions_and_quotes_spaces() {
    let (mut c, _) = composer();
    c.set_message("[$shared](skill://system:two) ".into());
    c.handle_paste("large paste ".repeat(1000));
    c.insert_str(" @ma");
    c.on_file_search_result(
        "ma".into(),
        vec![FileMatch {
            path: "my folder/main.rs".into(),
            root: "/work".into(),
            primary: true,
            indices: vec![],
            score: 100,
            is_dir: false,
        }],
    );
    key(&mut c, KeyCode::Tab);
    assert!(c
        .current_text_with_pending()
        .starts_with("[$shared](skill://system:two) "));
    assert!(c
        .current_text_with_pending()
        .contains(&"large paste ".repeat(1000)));
    assert!(c.textarea.text().ends_with("\"my folder/main.rs\" "));
    assert_eq!(c.pending_pastes.len(), 1);
}

#[test]
fn image_completion_uses_search_root_and_preserves_existing_bindings() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("cat.png");
    image::RgbaImage::new(2, 2).save(&file).unwrap();
    let (mut c, _) = composer();
    c.set_message("[$shared](skill://system:two) @cat".into());
    c.on_file_search_result(
        "cat".into(),
        vec![FileMatch {
            path: "cat.png".into(),
            root: home.path().into(),
            primary: true,
            indices: vec![],
            score: 100,
            is_dir: false,
        }],
    );
    key(&mut c, KeyCode::Tab);
    assert_eq!(c.textarea.text(), "$shared [Image #1] ");
    assert_eq!(c.images.len(), 1);
    assert_eq!(
        c.current_text_with_pending(),
        "[$shared](skill://system:two) [Image #1] "
    );
}

#[test]
fn successive_history_navigation_uses_visible_labels_and_restores_exact_bindings() {
    let (mut c, _) = composer();
    for text in [
        "[$first](skill://system:one)",
        "[$second](skill://system:two)",
    ] {
        c.history.record_local_submission(text.into());
    }
    key(&mut c, KeyCode::Up);
    assert_eq!(c.textarea.text(), "$second");
    key(&mut c, KeyCode::Up);
    assert_eq!(c.textarea.text(), "$first");
    key(&mut c, KeyCode::Down);
    assert_eq!(c.textarea.text(), "$second");
    assert_eq!(
        c.current_text_with_pending(),
        "[$second](skill://system:two)"
    );
    assert!(matches!(c.active_popup, ActivePopup::None));
}
