//! LlmService — thin, lock-free wrapper around an optional `LLMClient`.
//!
//! `Box<dyn LLMClient>` is `Send + Sync` (the trait bound requires it), so
//! no interior-mutability wrapper is needed — all operations are `&self`.

use crate::llm::{ChatMessage, LLMClient};
use tracing::warn;

pub struct LlmService {
    client: Option<Box<dyn LLMClient>>,
    model: String,
}

impl LlmService {
    pub fn new(client: Option<Box<dyn LLMClient>>, model: impl Into<String>) -> Self {
        Self {
            client,
            model: model.into(),
        }
    }

    pub fn has_client(&self) -> bool {
        self.client.is_some()
    }

    pub fn client(&self) -> Option<&dyn LLMClient> {
        self.client.as_ref().map(|c| c.as_ref())
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn ask(
        &self,
        prompt: &str,
        system: &str,
        temperature: f64,
        max_tokens: u32,
    ) -> Option<String> {
        let client = self.client.as_ref()?;
        let messages = vec![ChatMessage::system(system), ChatMessage::user(prompt)];
        match client
            .chat(messages, &self.model, temperature, max_tokens)
            .await
        {
            Ok(resp) => Some(resp.content),
            Err(e) => {
                warn!("LLM call failed: {}", e);
                None
            }
        }
    }

    pub async fn ask_messages(
        &self,
        messages: Vec<ChatMessage>,
        temperature: f64,
        max_tokens: u32,
    ) -> Option<String> {
        let client = self.client.as_ref()?;
        match client
            .chat(messages, &self.model, temperature, max_tokens)
            .await
        {
            Ok(resp) => Some(resp.content),
            Err(e) => {
                warn!("LLM chat failed: {}", e);
                None
            }
        }
    }
}
