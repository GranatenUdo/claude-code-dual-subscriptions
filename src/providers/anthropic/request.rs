pub(super) use crate::providers::messages_request::prepare_body;

pub(crate) fn resolve_model(model: &str) -> &str {
    match model {
        "haiku" => "claude-haiku-4-5",
        "sonnet" => "claude-sonnet-5",
        "opus" => "claude-opus-5-5",
        "fable" => "claude-fable-5",
        _ => model,
    }
}
