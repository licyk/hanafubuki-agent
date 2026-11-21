//! What an answer written by an earlier version reads as. A record is
//! decoded tolerantly: one that names only an option, or only words, or
//! neither, still opens the session it belongs to.

use hanafubuki_agent_kernel::{
    AgentUserInputAnswer, AgentUserInputQuestion, AgentUserInputRequest, AgentUserInputResponse,
};

fn answer(json: serde_json::Value) -> AgentUserInputAnswer {
    serde_json::from_value(json).unwrap()
}

#[test]
fn an_answer_that_names_an_option_reads_as_that_option() {
    let read = answer(serde_json::json!({
        "questionId": "scope",
        "response": {"kind": "option", "optionId": "focused"}
    }));

    assert_eq!(
        read.response,
        AgentUserInputResponse::Option {
            option_id: "focused".to_string()
        }
    );
}

#[test]
fn an_answer_that_names_only_an_option_reads_as_that_option() {
    // How versions before free-form answers wrote it.
    let read = answer(serde_json::json!({"questionId": "scope", "optionId": "focused"}));

    assert_eq!(
        read.response,
        AgentUserInputResponse::Option {
            option_id: "focused".to_string()
        }
    );
}

#[test]
fn an_answer_in_the_persons_words_reads_as_those_words() {
    let read = answer(serde_json::json!({
        "questionId": "why",
        "response": {"kind": "text", "text": "because it is safer"}
    }));

    assert_eq!(
        read.response,
        AgentUserInputResponse::Text {
            text: "because it is safer".to_string()
        }
    );
}

#[test]
fn an_answer_with_nothing_in_it_still_reads() {
    let read = answer(serde_json::json!({"questionId": "scope"}));

    assert_eq!(
        read.response,
        AgentUserInputResponse::Text {
            text: String::new()
        },
        "an empty answer, not a refusal to open the session"
    );
}

#[test]
fn a_request_written_before_blocking_was_a_field_reads_as_blocking() {
    let request: AgentUserInputRequest = serde_json::from_value(serde_json::json!({
        "requestId": 1,
        "callId": "call-1",
        "questions": [],
        "requestedAt": 0
    }))
    .unwrap();

    assert!(request.is_blocking);
}

#[test]
fn a_question_written_before_free_form_answers_takes_none() {
    let question: AgentUserInputQuestion = serde_json::from_value(serde_json::json!({
        "id": "scope",
        "header": "Scope",
        "question": "Which?",
        "options": []
    }))
    .unwrap();

    assert!(!question.is_other);
}
