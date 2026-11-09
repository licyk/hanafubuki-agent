// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the host's `decompress` schema extension in `src/compress-tool.ts`
// (`withRangeParams`, `withImageRestoreParam`, `DECOMPRESS_IMAGE_NOTE`) at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: range restore needs no content store (the
// Agent keeps the full history), so `startId`/`endId` are always offered and
// their text names no setting; restored images are attached to the result
// instead of written to files.

//! The schemas of the compression tools: billion-context's kernel schemas,
//! verbatim, with its host's `decompress` extension layered on top.

use hanafubuki_agent_model_api::ModelToolSpec;
use serde_json::{json, Value};

use super::AgentCoreTool;

const DECOMPRESS_RANGE_START: &str = "Optional mNNNNN message ref, inclusive lower bound of a sub-range of this block. With endId, restores only that span instead of the whole block. Omit both to restore the whole block; empty values count as omitted.";
const DECOMPRESS_RANGE_END: &str = "Optional mNNNNN message ref, inclusive upper bound. Used together with startId; omit both to restore the whole block.";
const DECOMPRESS_IMAGE: &str = "Optional mNNNNN message ref whose IMAGE payload you want restored (an image that was folded away), or a handle a summary's MEDIA REFS lists. Pass \"list\" to enumerate every image currently restorable. Restored images are attached to the result. Independent of blockId/range.";
const DECOMPRESS_IMAGE_NOTE: &str = " Pass imageRef (an mNNNNN ref) to restore a folded image's original pixels, attached to the result; imageRef:\"list\" shows what is restorable.";

pub(super) fn compression_spec(tool: AgentCoreTool) -> ModelToolSpec {
    let schema = hanafubuki_agent_compression::acp_tools()
        .into_iter()
        .find(|schema| schema.name == tool.name())
        .expect("every compression core tool has a schema");
    let mut spec = ModelToolSpec {
        strict: false,
        name: schema.name.to_owned(),
        description: schema.description.to_owned(),
        input_schema: schema.parameters,
    };
    if tool == AgentCoreTool::Decompress {
        spec.description.push_str(DECOMPRESS_IMAGE_NOTE);
        add_properties(
            &mut spec.input_schema,
            [
                ("startId", DECOMPRESS_RANGE_START),
                ("endId", DECOMPRESS_RANGE_END),
                ("imageRef", DECOMPRESS_IMAGE),
            ],
        );
    }
    spec
}

fn add_properties<const N: usize>(schema: &mut Value, properties: [(&str, &str); N]) {
    let Some(object) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    for (name, description) in properties {
        object.insert(
            name.to_owned(),
            json!({ "type": "string", "description": description }),
        );
    }
}
