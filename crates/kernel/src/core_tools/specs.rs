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
            AgentCoreTool::RequestUserInput => ModelToolSpec { strict: true,
                name: tool.name().to_owned(),
                description: "Pause the turn and ask the user one to three structured questions.".to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "questions": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 3,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string", "minLength": 1, "maxLength": 64 },
                                    "header": { "type": "string", "minLength": 1, "maxLength": 64 },
                                    "question": { "type": "string", "minLength": 1, "maxLength": 1024 },
                                    "options": {
                                        "type": "array",
                                        "minItems": 2,
                                        "maxItems": 3,
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
