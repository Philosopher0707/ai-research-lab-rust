//! Single source of truth for all LLM provider metadata.
//!
//! Every match arm across config.rs and llm/client.rs that mapped a provider
//! string to a URL, default model, or env-key is replaced by this table.

/// Static metadata for one LLM provider.
pub struct ProviderDef {
    /// Canonical short ID used in config files and env vars (e.g. `"anthropic"`).
    pub id: &'static str,
    /// Human-readable name shown in CLIs and docs.
    pub display_name: &'static str,
    /// Base URL for the REST API (Anthropic native or OpenAI-compatible).
    pub base_url: &'static str,
    /// Model used when the caller doesn't supply one.
    pub default_model: &'static str,
    /// Environment variable that holds the API key (empty string = keyless).
    pub env_key: &'static str,
    /// Whether the provider works without an API key (e.g. local Ollama).
    pub keyless: bool,
    /// Whether to use the Anthropic messages API rather than OpenAI-compatible.
    pub anthropic_api: bool,
}

/// All supported providers. Order matters for `detect_from_env()` — first match wins.
pub const PROVIDERS: &[ProviderDef] = &[
    ProviderDef {
        id: "anthropic",
        display_name: "Anthropic",
        base_url: "https://api.anthropic.com",
        default_model: "claude-sonnet-4-6",
        env_key: "ANTHROPIC_API_KEY",
        keyless: false,
        anthropic_api: true,
    },
    ProviderDef {
        id: "openai",
        display_name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-4o",
        env_key: "OPENAI_API_KEY",
        keyless: false,
        anthropic_api: false,
    },
    ProviderDef {
        id: "openrouter",
        display_name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        default_model: "anthropic/claude-sonnet-4-5",
        env_key: "OPENROUTER_API_KEY",
        keyless: false,
        anthropic_api: false,
    },
    ProviderDef {
        id: "deepseek",
        display_name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-chat",
        env_key: "DEEPSEEK_API_KEY",
        keyless: false,
        anthropic_api: false,
    },
    ProviderDef {
        id: "zhipu",
        display_name: "Zhipu AI",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        default_model: "glm-4-flash",
        env_key: "ZHIPU_API_KEY",
        keyless: false,
        anthropic_api: false,
    },
    ProviderDef {
        id: "minimax",
        display_name: "MiniMax",
        base_url: "https://api.minimax.chat/v1",
        default_model: "MiniMax-Text-01",
        env_key: "MINIMAX_API_KEY",
        keyless: false,
        anthropic_api: false,
    },
    ProviderDef {
        id: "xai",
        display_name: "xAI",
        base_url: "https://api.x.ai/v1",
        default_model: "grok-2",
        env_key: "XAI_API_KEY",
        keyless: false,
        anthropic_api: false,
    },
    ProviderDef {
        id: "local",
        display_name: "Local (Ollama)",
        base_url: "http://localhost:11434/v1",
        default_model: "llama3",
        env_key: "",
        keyless: true,
        anthropic_api: false,
    },
];

/// Look up a provider by its ID (e.g. `"anthropic"`, `"openai"`).
/// Returns `None` for unknown IDs — callers should fall back to a sensible default.
pub fn find(id: &str) -> Option<&'static ProviderDef> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Scan environment variables and return the first provider whose key is set.
/// Useful for zero-config auto-detection when no provider is explicitly configured.
pub fn detect_from_env() -> Option<&'static ProviderDef> {
    PROVIDERS
        .iter()
        .find(|p| !p.keyless && !p.env_key.is_empty() && std::env::var(p.env_key).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_providers_have_defaults() {
        for p in PROVIDERS {
            assert!(!p.id.is_empty(), "provider has empty id");
            assert!(
                !p.base_url.is_empty() || p.keyless,
                "provider {} missing base_url",
                p.id
            );
            assert!(
                !p.default_model.is_empty(),
                "provider {} missing default_model",
                p.id
            );
        }
    }

    #[test]
    fn find_known_providers() {
        assert!(find("anthropic").is_some());
        assert!(find("openai").is_some());
        assert!(find("local").map(|p| p.keyless) == Some(true));
        assert!(find("nonexistent").is_none());
    }
}
