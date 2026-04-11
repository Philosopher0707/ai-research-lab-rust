//! LLM API client trait + OpenAI-compatible and Anthropic implementations.
//!
//! All provider metadata (base URL, default model, env key) is sourced from
//! `crate::providers` — there are no duplicate match tables here.

use crate::errors::LLMError;
use crate::providers;
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
    ) -> Result<ChatResponse, LLMError>;
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
    ) -> Result<ChatResponse, LLMError> {
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

        // Retry with exponential backoff for network errors and 429s.
        const MAX_RETRIES: u32 = 3;
        let mut last_err = LLMError::Network("no attempts made".into());

        for attempt in 1..=MAX_RETRIES {
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
                        .map_err(|e| LLMError::MalformedResponse(e.to_string()))?;

                    if status < 400 {
                        let choice =
                            body.get("choices").and_then(|c| c.get(0)).ok_or_else(|| {
                                LLMError::MalformedResponse(format!(
                                    "no choices in response: {}",
                                    body
                                ))
                            })?;
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

                    let err_msg = body
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                        .to_string();

                    if status == 429 {
                        if attempt < MAX_RETRIES {
                            let delay = 2f64.powi(attempt as i32 - 1);
                            tracing::warn!(
                                "Rate limited (attempt {}/{}), retrying in {:.1}s",
                                attempt,
                                MAX_RETRIES,
                                delay
                            );
                            tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                            last_err = LLMError::RateLimited;
                            continue;
                        }
                        return Err(LLMError::RateLimited);
                    }

                    if status == 402 || status == 429 {
                        return Err(LLMError::BudgetExceeded);
                    }

                    return Err(LLMError::ApiError {
                        status,
                        message: err_msg,
                    });
                }
                Err(e) => {
                    last_err = LLMError::Network(e.to_string());
                    if attempt < MAX_RETRIES {
                        let delay = 2f64.powi(attempt as i32 - 1);
                        tracing::warn!(
                            "Request failed (attempt {}/{}): {}. Retrying in {:.1}s",
                            attempt,
                            MAX_RETRIES,
                            last_err,
                            delay
                        );
                        tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                    }
                }
            }
        }

        Err(last_err)
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
    ) -> Result<ChatResponse, LLMError> {
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
            .map_err(|e| LLMError::Network(e.to_string()))?;

        let status = resp.status().as_u16();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| LLMError::MalformedResponse(e.to_string()))?;

        if status == 429 {
            return Err(LLMError::RateLimited);
        }
        if status >= 400 {
            let err_msg = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string();
            return Err(LLMError::ApiError {
                status,
                message: err_msg,
            });
        }

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
                if let Some(v) = u.get("input_tokens").and_then(|v| v.as_u64()) {
                    map.insert("input_tokens".into(), v);
                }
                if let Some(v) = u.get("output_tokens").and_then(|v| v.as_u64()) {
                    map.insert("output_tokens".into(), v);
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

/// Resolve the best API key for the given provider.
/// Checks the provider-specific env var first, then the generic `LAB_API_KEY`.
fn resolve_api_key(provider: &str, explicit_key: &str) -> String {
    if !explicit_key.is_empty() {
        return explicit_key.to_string();
    }
    let env_key = providers::find(provider)
        .filter(|p| !p.env_key.is_empty())
        .map(|p| p.env_key);

    env_key
        .and_then(|k| std::env::var(k).ok())
        .or_else(|| std::env::var("LAB_API_KEY").ok())
        .unwrap_or_default()
}

/// Build an LLM client for the given provider.
///
/// All non-Anthropic providers use the OpenAI-compatible chat completions format.
/// Provider metadata (base URL, default model) is sourced from `providers::PROVIDERS`.
pub fn create_client(
    provider: &str,
    api_key: &str,
    model: &str,
    base_url: &str,
) -> Box<dyn LLMClient> {
    let api_key = resolve_api_key(provider, api_key);

    let def = providers::find(provider);
    let model = if model.is_empty() {
        def.map(|p| p.default_model).unwrap_or("gpt-4o")
    } else {
        model
    };

    if def.map(|p| p.anthropic_api).unwrap_or(false) {
        return Box::new(AnthropicClient::new(api_key));
    }

    let url = if base_url.is_empty() {
        def.map(|p| p.base_url)
            .unwrap_or("https://openrouter.ai/api/v1")
    } else {
        base_url
    };

    Box::new(OpenAICompatibleClient::new(
        api_key,
        url.to_string(),
        model.to_string(),
    ))
}
