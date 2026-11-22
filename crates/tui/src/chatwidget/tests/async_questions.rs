use super::*;
use hanafubuki_agent::AgentAsyncQuestion;

#[test]
fn async_questions_skip_dismiss_external_replies_and_ignore_replayed_calls() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    let ask = |call: &str| {
        TurnUpdate::AsyncQuestions(
            call.into(),
            vec![
                AgentAsyncQuestion {
                    title: "First?".into(),
                    options: None,
                },
                AgentAsyncQuestion {
                    title: "Second?".into(),
                    options: None,
                },
            ],
        )
    };
    chat.handle_turn_update(TurnUpdate::SteeringConsumed(
        1,
        vec![AgentAsyncQuestion::item_id("later", 0)],
    ));
    chat.handle_turn_update(ask("later"));
    chat.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    assert!(screen(&chat, 80).contains("Second?"));
    chat.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    chat.handle_turn_update(ask("later"));
    assert!(!screen(&chat, 80).contains("to answer"));
    chat.handle_turn_update(ask("live"));
    chat.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    chat.handle_turn_update(TurnUpdate::SteeringConsumed(
        2,
        vec![AgentAsyncQuestion::item_id("live", 0)],
    ));
    assert!(screen(&chat, 80).contains("Second?"));
    chat.handle_turn_update(TurnUpdate::SteeringConsumed(3, vec!["live".into()]));
    assert!(!screen(&chat, 80).contains("Question 1"));
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
    chat.handle_turn_update(ask("live"));
    assert!(!screen(&chat, 80).contains("to answer"));
    assert!(drain(&events).1.is_empty());
}

#[test]
fn async_questions_require_painting_a_whole_suggestion_before_it_can_be_sent() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    let option = (1..=20)
        .map(|i| format!("Line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    chat.handle_turn_update(TurnUpdate::AsyncQuestions(
        "long".into(),
        vec![AgentAsyncQuestion {
            title: "Proceed?".into(),
            options: Some(vec![option.clone()]),
        }],
    ));
    chat.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    let view = Rect::new(0, 0, 40, 7);
    let paint = |chat: &ChatWidget| {
        chat.bottom_pane
            .command_view("request_user_input_async")
            .unwrap()
            .render(view, &mut Buffer::empty(view))
    };
    paint(&chat);
    chat.handle_key_event(key(KeyCode::Enter));
    assert!(drain(&events).1.is_empty());
    for _ in 0..12 {
        paint(&chat);
        chat.handle_key_event(key(KeyCode::PageDown));
    }
    chat.handle_key_event(key(KeyCode::Enter));
    let ops = drain(&events).1;
    let [Op::Steer { message, .. }] = ops.as_slice() else {
        panic!("Unexpected operations: {ops:?}")
    };
    assert_eq!(
        hanafubuki_agent::AgentAsyncQuestionReply::from_message(&message.text)[0].answer,
        option
    );
}
