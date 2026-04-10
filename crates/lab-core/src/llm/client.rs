//! LLM API client trait + OpenAI-compatible and Anthropic implementations.
//! Mirrors core/llm/client.py (301 lines)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── ChatResponse ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    #[serde(default = "default_stop")]
    pub finish_reason: String,
    #[serde(default)]
    pub usage: HashMap<String, u64>,
    #[serde(skip)]
    pub raw: Option<serde_json::Value>,
}

fn default_stop() -> String {
    "stop".into()
}

// ─── LLMClient Trait ───────────────────────────────────────────────

#[async_trait]
pub trait LLMClient: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        model: &str,
        temperature: f64,
        max_tokens: u32,
    ) -> Result<ChatResponse, String>;
}

// ─── ChatMessage ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

// ─── OpenAI-Compatible Client ──────────────────────────────────────

pub struct OpenAICompatibleClient {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    client: reqwest::Client,
}

impl OpenAICompatibleClient {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        Self {
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl LLMClient for OpenAICompatibleClient {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        model: &str,
        temperature: f64,
        max_tokens: u32,
    ) -> Result<ChatResponse, String> {
        let model = if model.is_empty() { &self.model } else { model };

        let payload = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        if !self.api_key.is_empty() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.api_key).parse().unwrap(),
            );
        }
        headers.insert(
            "HTTP-Referer",
            "https://github.com/ai-research-lab".parse().unwrap(),
        );
        headers.insert("X-Title", "AI Research Lab".parse().unwrap());

        let url = format!("{}/chat/completions", self.base_url);

        // Retry with exponential backoff for 429
        let max_retries = 3;
        let mut last_error = String::new();

        for attempt in 1..=max_retries {
            let resp = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&payload)
                .send()
                .await;

            match resp {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body: serde_json::Value = resp
                        .json()
                        .await
                        .map_err(|e| format!("Failed to parse response: {}", e))?;

                    if status < 400 {
                        // Parse response
                        if let Some(choice) = body.get("choices").and_then(|c| c.get(0)) {
                            let content = choice
                                .get("message")
                                .and_then(|m| m.get("content"))
                                .and_then(|c| c.as_str())
                                .unwrap_or("")
                                .to_string();
                            let finish_reason = choice
                                .get("finish_reason")
                                .and_then(|r| r.as_str())
                                .unwrap_or("stop")
                                .to_string();
                            let usage: HashMap<String, u64> = body
                                .get("usage")
                                .map(|u| serde_json::from_value(u.clone()).unwrap_or_default())
                                .unwrap_or_default();

                            return Ok(ChatResponse {
                                content,
                                model: body
                                    .get("model")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or(model)
                                    .to_string(),
                                finish_reason,
                                usage,
                                raw: Some(body),
                            });
                        }
                        return Err(format!("No choices in response: {}", body));
                    }

                    if status == 429 && attempt < max_retries {
                        let delay = 2f64.powi(attempt as i32 - 1);
                        tracing::warn!(
                            "Rate limited (attempt {}/{}), retrying in {:.1}s",
                            attempt,
                            max_retries,
                            delay
                        );
                        tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                        continue;
                    }

                    let err_msg = body
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown error");
                    return Err(format!("LLM API error {}: {}", status, err_msg));
                }
                Err(e) => {
                    last_error = e.to_string();
                    if attempt < max_retries {
                        let delay = 2f64.powi(attempt as i32 - 1);
                        tracing::warn!(
                            "Request failed (attempt {}/{}): {}. Retrying in {:.1}s",
                            attempt,
                            max_retries,
                            last_error,
                            delay
                        );
                        tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                    }
                }
            }
        }

        Err(format!(
            "LLM API request failed after {} attempts: {}",
            max_retries, last_error
        ))
    }
}

// ─── Anthropic Client ──────────────────────────────────────────────

pub struct AnthropicClient {
    pub api_key: String,
    client: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl LLMClient for AnthropicClient {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        model: &str,
        temperature: f64,
        max_tokens: u32,
    ) -> Result<ChatResponse, String> {
        // Convert messages: extract system, rest go to messages array
        let mut system_msg = String::new();
        let mut anthropic_messages = Vec::new();
        for msg in &messages {
            if msg.role == "system" {
                system_msg = msg.content.clone();
            } else {
                anthropic_messages.push(serde_json::json!({
                    "role": msg.role,
                    "content": msg.content,
                }));
            }
        }

        let mut payload = serde_json::json!({
            "model": model,
            "messages": anthropic_messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });
        if !system_msg.is_empty() {
            payload["system"] = serde_json::json!(system_msg);
        }

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        headers.insert("x-api-key", self.api_key.parse().unwrap());
        headers.insert("anthropic-version", "2023-06-01".parse().unwrap());

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Anthropic API request failed: {}", e))?;

        let status = resp.status().as_u16();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if status >= 400 {
            let err_msg = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return Err(format!("Anthropic API error {}: {}", status, err_msg));
        }

        // Parse content blocks
        let mut content = String::new();
        if let Some(blocks) = body.get("content").and_then(|c| c.as_array()) {
            for block in blocks {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        content.push_str(text);
                    }
                }
            }
        }

        let usage: HashMap<String, u64> = body
            .get("usage")
            .map(|u| {
                let mut map = HashMap::new();
                if let Some(input) = u.get("input_tokens").and_then(|v| v.as_u64()) {
                    map.insert("input_tokens".into(), input);
                }
                if let Some(output) = u.get("output_tokens").and_then(|v| v.as_u64()) {
                    map.insert("output_tokens".into(), output);
                }
                map
            })
            .unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: body
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or(model)
                .to_string(),
            finish_reason: body
                .get("stop_reason")
                .and_then(|r| r.as_str())
                .unwrap_or("end_turn")
                .to_string(),
            usage,
            raw: Some(body),
        })
    }
}

// ─── Factory ───────────────────────────────────────────────────────

/// Provider default base URLs (used when base_url is empty).
fn provider_base_url(provider: &str) -> &'static str {
    match provider {
        "openai" => "https://api.openai.com/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "deepseek" => "https://api.deepseek.com/v1",
        "zhipu" => "https://open.bigmodel.cn/api/paas/v4",
        "minimax" => "https://api.minimax.chat/v1",
        "xai" => "https://api.x.ai/v1",
        "local" => "http://localhost:11434/v1",
        _ => "https://api.openai.com/v1",
    }
}

/// Provider default models (used when model is empty).
fn provider_default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-4-6",
        "openai" => "gpt-4o",
        "openrouter" => "anthropic/claude-sonnet-4-5",
        "deepseek" => "deepseek-chat",
        "zhipu" => "glm-4-flash",
        "minimax" => "MiniMax-Text-01",
        "xai" => "grok-2",
        _ => "gpt-4o",
    }
}

/// Resolve the best api_key for the given provider.
/// Tries provider-specific env vars first, then generic LAB_API_KEY.
fn resolve_api_key(provider: &str, explicit_key: &str) -> String {
    if !explicit_key.is_empty() {
        return explicit_key.to_string();
    }
    let from_env = match provider {
        "anthropic" => std::env::var("ANTHROPIC_API_KEY").ok(),
        "openai" => std::env::var("OPENAI_API_KEY").ok(),
        "openrouter" => std::env::var("OPENROUTER_API_KEY").ok(),
        "deepseek" => std::env::var("DEEPSEEK_API_KEY").ok(),
        "zhipu" => std::env::var("ZHIPU_API_KEY").ok(),
        "minimax" => std::env::var("MINIMAX_API_KEY").ok(),
        "xai" => std::env::var("XAI_API_KEY").ok(),
        _ => None,
    };
    from_env
        .or_else(|| std::env::var("LAB_API_KEY").ok())
        .unwrap_or_default()
}

/// Build an LLM client for the given provider.
///
/// Supported providers:
///   anthropic | openai | openrouter | deepseek | zhipu | minimax | xai | local
///
/// All non-Anthropic providers use the OpenAI-compatible chat completions format.
pub fn create_client(
    provider: &str,
    api_key: &str,
    model: &str,
    base_url: &str,
) -> Box<dyn LLMClient> {
    let api_key = resolve_api_key(provider, api_key);
    let model = if model.is_empty() {
        provider_default_model(provider)
    } else {
        model
    };

    match provider {
        "anthropic" => Box::new(AnthropicClient::new(api_key)),
        _ => {
            let url = if base_url.is_empty() {
                provider_base_url(provider)
            } else {
                base_url
            };
            Box::new(OpenAICompatibleClient::new(
                api_key,
                url.to_string(),
                model.to_string(),
            ))
        }
    }
}
