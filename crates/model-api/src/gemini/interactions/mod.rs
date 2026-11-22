mod decode;
mod encode;
mod output;
mod step;

pub(super) use decode::Decoder;
pub(super) use encode::encode;

pub(super) fn stream_error(frame: &serde_json::Value) -> Option<&serde_json::Value> {
    if frame["error"].is_object() {
        return Some(&frame["error"]);
    }
    if frame["event_type"] == "error" {
        return Some(frame);
    }
    let interaction = &frame["interaction"];
    if frame["event_type"] != "interaction.completed" || interaction["status"] != "failed" {
        return None;
    }
    interaction
        .get("error")
        .filter(|error| error.is_object())
        .or_else(|| {
            interaction["errors"]
                .as_array()?
                .iter()
                .find(|error| error["message"].is_string() || error["code"].is_string())
        })
}

#[cfg(test)]
mod tests;
