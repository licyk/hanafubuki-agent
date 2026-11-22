// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).
//! Optional questions and the user message identifying an answer across clients.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAsyncQuestion {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

impl AgentAsyncQuestion {
    /// Codex's quoted model-authored framing occupies at most 512 bytes.
    pub fn quoted_title(question: &str) -> String {
        question[..question.floor_char_boundary(question.len().min(512))].replace(['\n', '\r'], " ")
    }

    pub fn item_id(call_id: &str, index: usize) -> String {
        serde_json::json!(["request_user_input_async", call_id, index]).to_string()
    }
}

const OPEN: &str = "<send_user_message_question_reply>";
const CLOSE: &str = "</send_user_message_question_reply>";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAsyncQuestionReply {
    pub question_item_id: String,
    pub question: String,
    pub answer: String,
}

impl AgentAsyncQuestionReply {
    /// Codex's reply envelope bounds model-authored framing to 512 bytes and
    /// keeps the user's entire answer. An oversized identity uses the quoted form.
    pub fn message(item_id: &str, question: &str, answer: &str) -> String {
        let question = AgentAsyncQuestion::quoted_title(question);
        if item_id.len() > 512 {
            return format!("> {question}\n\n{answer}");
        }
        let replies =
            serde_json::json!([{"answer":answer, "question":question, "questionItemId":item_id}]);
        format!("{OPEN}\n{replies}\n{CLOSE}")
    }

    /// Recognizes only complete reply messages. A malformed entry does not
    /// hide another client's usable answers in the same envelope.
    pub fn from_message(text: &str) -> Vec<Self> {
        let Some(body) = text
            .trim()
            .strip_prefix(OPEN)
            .and_then(|s| s.strip_suffix(CLOSE))
        else {
            return Vec::new();
        };
        serde_json::from_str::<Vec<serde_json::Value>>(body)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect()
    }
}
