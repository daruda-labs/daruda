use crate::strings as s;

/// A model choice's display label. The default-model entry carries a fixed
/// "Default (recommended)" name and names the model it resolves to only in its
/// description, so that model is shown instead.
pub(crate) fn agent_model_choice_label(
    value: &str,
    name: &str,
    description: Option<&str>,
) -> String {
    match daruda_acp::resolved_default_model(value, description) {
        Some(model) => s::agent_chat::config_default_model(model),
        None => name.to_string(),
    }
}
