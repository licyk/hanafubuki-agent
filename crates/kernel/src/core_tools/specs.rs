// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).
//! The model-visible specifications of the kernel's own tools.

use super::compression_spec::compression_spec;
use super::AgentCoreTool;
use hanafubuki_agent_model_api::ModelToolSpec;

pub(super) fn model_spec(tool: AgentCoreTool) -> ModelToolSpec {
    match tool {
            AgentCoreTool::NewContext => ModelToolSpec {
                strict: false,
                name: tool.name().to_owned(),
                description: "Request a fresh context window after this response is fully committed and the turn needs another sampling step.".to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            },
            AgentCoreTool::UpdatePlan => ModelToolSpec { strict: true,
                name: tool.name().to_owned(),
                description: "Replace the current turn's complete ordered task checklist.".to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "explanation": { "type": ["string", "null"], "maxLength": 2048 },
                        "plan": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 20,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "step": { "type": "string", "minLength": 1, "maxLength": 1024 },
                                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                                },
                                "required": ["step", "status"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["explanation", "plan"],
                    "additionalProperties": false
                }),
            },
            AgentCoreTool::RequestUserInputAsync => ModelToolSpec {
                strict: false,
                name: tool.name().to_owned(),
                description: "Ask the user one or more questions during ongoing work. Use this tool only to request missing information, preferences, constraints, clarification, or approval. The tool returns immediately without ending the turn or waiting for a reply; any reply arrives asynchronously as a new user message. Keep questions concise, self-contained, and easy to understand, using a level of detail appropriate to the user and task. The UI always allows a free-text answer, including when suggested options are provided. A preselected option is not submitted automatically.".into(),
                input_schema: serde_json::json!({
                    "type":"object", "properties":{
                        "questions":{"type":"array", "description":"One or more self-contained questions to present together, in display order.", "minItems":1, "items":{
                            "type":"object", "properties":{
                                "title":{"type":"string","description":"The complete question shown to the user, including any context needed to answer it."},
                                "options":{"type":"array","minItems":1,"items":{"type":"string"},"description":"Suggested answers, in display order. Put the recommended answer first; the first option is preselected by default. The user can select one option or enter a free-text answer. Do not include an Other option or a free-text placeholder; the UI provides free-text input automatically. Omit options for a free-text-only question."}
                            }, "required":["title"], "additionalProperties":false
                        }}
                    }, "required":["questions"], "additionalProperties":false
                }),
            },
            AgentCoreTool::RequestUserInput => ModelToolSpec { strict: true,
                name: tool.name().to_owned(),
                description: "Pause the turn and ask the user up to ten structured questions, each \
                              with up to five options. A question whose `isOther` is true also \
                              accepts an answer in the user's own words."
                    .to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "isBlocking": {
                            "type": "boolean",
                            "description": "Whether the turn waits for the answers (default true); when false the questions are shown and the turn goes on, and the person's answers reach it if they come"
                        },
                        "questions": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 10,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string", "minLength": 1, "maxLength": 64 },
                                    "header": { "type": "string", "minLength": 1, "maxLength": 64 },
                                    "question": { "type": "string", "minLength": 1, "maxLength": 1024 },
                                    "isOther": {
                                        "type": "boolean",
                                        "description": "Whether the user may answer in their own words too"
                                    },
                                    "options": {
                                        "type": "array",
                                        "minItems": 2,
                                        "maxItems": 5,
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "id": { "type": "string", "minLength": 1, "maxLength": 64 },
                                                "label": { "type": "string", "minLength": 1, "maxLength": 128 },
                                                "description": { "type": "string", "maxLength": 512 }
                                            },
                                            "required": ["id", "label", "description"],
                                            "additionalProperties": false
                                        }
                                    }
                                },
                                "required": ["id", "header", "question", "options"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["questions"],
                    "additionalProperties": false
                }),
            },
            AgentCoreTool::Compress
            | AgentCoreTool::Decompress
            | AgentCoreTool::SearchContext
            | AgentCoreTool::AcpStatus
            | AgentCoreTool::AcpCache
            | AgentCoreTool::Absorb
            | AgentCoreTool::AcpRetrieve
            | AgentCoreTool::AcpRule => compression_spec(tool),
    }
}
