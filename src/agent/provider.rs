use agent_sdk::providers::AnthropicProvider;
use crate::config::ClawConfig;

/// Create an LlmProvider based on model name.
/// Returns a boxed provider (we use AnthropicProvider for now, with OpenAI and Gemini placeholders).
pub fn create_provider(model: &str, config: &ClawConfig) -> AnthropicProvider {
    // For now, all models go through Anthropic
    // Later we'll add OpenAI and Gemini routing
    let api_key = config.anthropic_api_key.clone()
        .expect("ANTHROPIC_API_KEY must be set");

    match model {
        "claude-haiku" | "haiku" => AnthropicProvider::haiku(api_key),
        "claude-opus" | "opus" => AnthropicProvider::opus(api_key),
        "claude-sonnet-4-5" | "sonnet-4-5" => AnthropicProvider::sonnet_45(api_key),
        _ => AnthropicProvider::new(api_key, model.to_string()),
    }
}
