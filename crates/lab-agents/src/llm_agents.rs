//! LLM-enhanced agents with heuristic fallback.
//!
//! These agents run the heuristic version first, then optionally
//! use an LLM to enhance the analysis. If LLM fails, the heuristic
//! result is returned as-is (graceful degradation).

use crate::researcher::ResearcherAgent;
use crate::reviewer::ReviewerAgent;
use crate::summarizer::SummarizerAgent;
use lab_core::llm::{ChatMessage, LLMClient};
use lab_core::types::AgentResult;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use tracing::warn;

// ─── LLM-enhanced ResearcherAgent ──────────────────────────

pub struct LlmResearcherAgent {
    base: ResearcherAgent,
    client: Box<dyn LLMClient>,
    model: String,
}

impl LlmResearcherAgent {
    pub fn new(base: ResearcherAgent, client: Box<dyn LLMClient>, model: String) -> Self {
        Self {
            base,
            client,
            model,
        }
    }

    pub fn id(&self) -> &str {
        self.base.id()
    }
    pub fn session_id(&self) -> &str {
        self.base.session_id()
    }
    pub fn state(&self) -> lab_core::types::AgentState {
        self.base.state()
    }

    pub async fn execute(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        task: &str,
        pattern: Option<&str>,
        path: Option<&str>,
        file_limit: Option<usize>,
    ) -> AgentResult {
        // Step 1: Run heuristic research
        let result = self
            .base
            .execute(
                registry, memory, task, pattern, path, file_limit, None, None,
            )
            .await;
        if !result.success {
            return result;
        }

        // Step 2: Enhance with LLM (best-effort, falls through to heuristic on error)
        if let Some(files) = result.data.get("files").and_then(|v| v.as_array()) {
            let mut llm_findings = Vec::new();
            for file_info in files.iter().take(5) {
                if let Some(fp) = file_info.get("path").and_then(|v| v.as_str()) {
                    let content_result = registry
                        .execute(
                            "read_file",
                            &HashMap::from([("path".into(), serde_json::json!(fp))]),
                        )
                        .await;

                    if content_result
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        if let Some(content) = content_result
                            .get("data")
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            let truncated = if content.len() > 4000 {
                                let end = content.floor_char_boundary(4000);
                                &content[..end]
                            } else {
                                content
                            };
                            let system = "\
You are a senior software architect analyzing a Rust/Python multi-agent codebase called \
AI Research Lab. For each file, identify: (1) its primary responsibility, (2) the key types \
or functions it exposes, (3) how it relates to other modules (dependencies, what depends on it). \
Be precise and technical. Avoid filler phrases like 'This file contains' — just state the facts.";
                            let prompt = format!("File: {fp}\n\n{truncated}");
                            match self
                                .client
                                .chat(
                                    vec![ChatMessage::system(system), ChatMessage::user(prompt)],
                                    &self.model,
                                    0.2,
                                    1024,
                                )
                                .await
                            {
                                Ok(response) => {
                                    llm_findings.push(serde_json::json!({
                                        "path": fp,
                                        "llm_analysis": response.content,
                                        "method": "llm",
                                    }));
                                }
                                Err(e) => {
                                    warn!("LLM error for {}: {}", fp, e);
                                    llm_findings.push(serde_json::json!({
                                        "path": fp,
                                        "method": "heuristic_fallback",
                                    }));
                                }
                            }
                        }
                    }
                }
            }

            // Build enhanced result (move from result.data)
            let mut data = result.data.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("llm_findings".into(), serde_json::json!(llm_findings));
                obj.insert("llm_enhanced".into(), serde_json::json!(true));
            }
            return AgentResult::ok(data);
        }

        result
    }
}

// ─── LLM-enhanced ReviewerAgent ────────────────────────────

pub struct LlmReviewerAgent {
    base: ReviewerAgent,
    client: Box<dyn LLMClient>,
    model: String,
}

impl LlmReviewerAgent {
    pub fn new(base: ReviewerAgent, client: Box<dyn LLMClient>, model: String) -> Self {
        Self {
            base,
            client,
            model,
        }
    }

    pub fn id(&self) -> &str {
        self.base.id()
    }
    pub fn session_id(&self) -> &str {
        self.base.session_id()
    }
    pub fn state(&self) -> lab_core::types::AgentState {
        self.base.state()
    }

    pub async fn execute(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        task: &str,
        pattern: Option<&str>,
        path: Option<&str>,
        file_limit: Option<usize>,
    ) -> AgentResult {
        let result = self
            .base
            .execute(
                registry, memory, task, pattern, path, file_limit, None, None,
            )
            .await;
        if !result.success {
            return result;
        }

        if let Some(reviews) = result.data.get("reviews").and_then(|v| v.as_array()) {
            let mut llm_reviews = Vec::new();
            for review in reviews.iter().take(3) {
                if let Some(fp) = review.get("path").and_then(|v| v.as_str()) {
                    let content_result = registry
                        .execute(
                            "read_file",
                            &HashMap::from([("path".into(), serde_json::json!(fp))]),
                        )
                        .await;

                    if content_result
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        if let Some(content) = content_result
                            .get("data")
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            let system = "\
You are a strict code reviewer for the AI Research Lab — a Rust/Python multi-agent workspace. \
Review the provided file for: unsafe unwrap/expect calls, missing error propagation, \
API design issues, performance problems (unnecessary clones, blocking in async context), \
missing documentation, and correctness bugs. \
Structure your response as: \
'## Issues' (numbered list with file:line references where possible) and \
'## Suggestions' (numbered list of improvements). \
Be terse and direct — engineering quality review, not a tutorial.";
                            let prompt = format!("File: {fp}\n\n```\n{content}\n```");
                            match self
                                .client
                                .chat(
                                    vec![ChatMessage::system(system), ChatMessage::user(prompt)],
                                    &self.model,
                                    0.2,
                                    2048,
                                )
                                .await
                            {
                                Ok(response) => {
                                    llm_reviews.push(serde_json::json!({
                                        "path": fp,
                                        "llm_review": response.content,
                                    }));
                                }
                                Err(e) => {
                                    warn!("LLM error for {}: {}", fp, e);
                                    llm_reviews.push(serde_json::json!({
                                        "path": fp,
                                        "llm_review": format!("LLM error: {e}"),
                                    }));
                                }
                            }
                        }
                    }
                }
            }

            let mut data = result.data.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("llm_reviews".into(), serde_json::json!(llm_reviews));
                obj.insert("llm_enhanced".into(), serde_json::json!(true));
            }
            return AgentResult::ok(data);
        }

        result
    }
}

// ─── LLM-enhanced SummarizerAgent ──────────────────────────

pub struct LlmSummarizerAgent {
    base: SummarizerAgent,
    client: Box<dyn LLMClient>,
    model: String,
}

impl LlmSummarizerAgent {
    pub fn new(base: SummarizerAgent, client: Box<dyn LLMClient>, model: String) -> Self {
        Self {
            base,
            client,
            model,
        }
    }

    pub fn id(&self) -> &str {
        self.base.id()
    }
    pub fn session_id(&self) -> &str {
        self.base.session_id()
    }
    pub fn state(&self) -> lab_core::types::AgentState {
        self.base.state()
    }

    pub async fn execute(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        task: &str,
        output_path: Option<&str>,
    ) -> AgentResult {
        // Run heuristic summary first
        let result = self
            .base
            .execute(registry, memory, task, output_path, None, None)
            .await;
        if !result.success {
            return result;
        }

        // LLM synthesis — gather data and ask LLM
        let mut context = String::new();
        if let Some(r) = memory.get(self.base.session_id(), "researcher_map") {
            context.push_str(&format!(
                "Research:\n{}\n\n",
                serde_json::to_string_pretty(&r).unwrap_or_default()
            ));
        }
        if let Some(r) = memory.get(self.base.session_id(), "review_results") {
            context.push_str(&format!(
                "Review:\n{}\n\n",
                serde_json::to_string_pretty(&r).unwrap_or_default()
            ));
        }

        if context.is_empty() {
            return result; // No data to synthesize
        }

        let system = "\
You are the executive synthesis agent for the AI Research Lab. You receive structured JSON \
data from researcher and reviewer agents and produce a concise technical summary for the \
engineering team. Your output must have exactly these sections: \
'## Codebase Overview' (2-3 sentences), \
'## Key Findings' (5 bullet points max), \
'## Risk Areas' (issues that need immediate attention), \
'## Recommended Actions' (prioritized, numbered list). \
Write in a direct, technical tone. Max 500 words total.";
        let prompt = format!("Agent findings:\n\n{context}");
        match self
            .client
            .chat(
                vec![ChatMessage::system(system), ChatMessage::user(prompt)],
                &self.model,
                0.2,
                2048,
            )
            .await
        {
            Ok(response) => {
                let enhanced_path = output_path.unwrap_or("lab-outputs/llm-summary.md");
                registry
                    .execute(
                        "write_file",
                        &HashMap::from([
                            ("path".into(), serde_json::json!(enhanced_path)),
                            ("content".into(), serde_json::json!(&response.content)),
                        ]),
                    )
                    .await;

                AgentResult::ok(serde_json::json!({
                    "task": task,
                    "llm_summary": response.content,
                    "llm_enhanced": true,
                    "output_path": enhanced_path,
                }))
            }
            Err(e) => {
                warn!("LLM synthesis failed: {e}, returning heuristic result");
                result
            }
        }
    }
}
