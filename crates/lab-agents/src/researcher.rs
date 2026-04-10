//! ResearcherAgent — discovers and analyses codebase structure.
//! With optional LLM for intelligent architecture analysis.

use crate::base::AgentImpl;
use lab_core::config::AgentProfile;
use lab_core::llm::ChatMessage;
use lab_core::llm::LLMClient;
use lab_core::types::{AgentResult, AgentState};
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ResearcherAgent {
    impl_: AgentImpl,
    pub workspace: PathBuf,
}

impl ResearcherAgent {
    pub fn new(id: String, session_id: String, profile: AgentProfile, workspace: PathBuf) -> Self {
        Self {
            impl_: AgentImpl::new(
                id.clone(),
                "researcher".into(),
                session_id,
                profile,
                workspace.clone(),
            ),
            workspace,
        }
    }

    pub fn id(&self) -> &str {
        self.impl_.id()
    }
    pub fn session_id(&self) -> &str {
        self.impl_.session_id()
    }
    pub fn state(&self) -> AgentState {
        self.impl_.state()
    }

    /// Execute the research task with optional LLM intelligence.
    ///
    /// Without LLM: discovers files, counts classes/functions (heuristic)
    /// With LLM: additionally analyzes architecture, patterns, and dependencies
    pub async fn execute(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        task: &str,
        pattern: Option<&str>,
        path: Option<&str>,
        file_limit: Option<usize>,
        // Optional LLM support
        llm: Option<&dyn LLMClient>,
        model: Option<&str>,
    ) -> AgentResult {
        if let Err(e) = self.impl_.start().await {
            return AgentResult::fail(e.to_string(), None);
        }

        let pattern = pattern.unwrap_or("**/*.py");

        // 1. Discover files
        let files_result = registry
            .execute(
                "glob_search",
                &HashMap::from([("pattern".into(), serde_json::json!(pattern))]),
            )
            .await;

        if !files_result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return AgentResult::fail("File discovery failed", Some(files_result));
        }

        let mut file_list: Vec<String> = files_result
            .get("data")
            .and_then(|d| d.get("matches"))
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let total_files = file_list.len();
        if let Some(p) = path {
            let p_lower = p.to_lowercase();
            file_list.retain(|f| f.to_lowercase().contains(&p_lower));
        }

        // 2. Analyse files
        let limit = file_limit.unwrap_or(50).min(200);
        let mut file_details = Vec::new();
        for fp in file_list.iter().take(limit) {
            let content_result = registry
                .execute(
                    "read_file",
                    &HashMap::from([
                        ("path".into(), serde_json::json!(fp)),
                        ("limit".into(), serde_json::json!(300)),
                    ]),
                )
                .await;

            if !content_result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            let content: String = content_result
                .get("data")
                .and_then(|d| d.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_owned();

            let lines: Vec<&str> = content.lines().collect();
            let total_lines: usize = content_result
                .get("data")
                .and_then(|d| d.get("total_lines"))
                .and_then(|n| n.as_u64())
                .map(|n| n as usize)
                .unwrap_or(lines.len());

            let class_names: Vec<String> = lines
                .iter()
                .filter(|l| {
                    let trimmed = l.trim();
                    trimmed.starts_with("class ")
                        || trimmed.starts_with("pub struct ")
                        || trimmed.starts_with("pub enum ")
                })
                .map(|l| l.trim().to_string())
                .collect();

            let async_count = lines
                .iter()
                .filter(|l| l.contains("async fn ") || l.contains("async def "))
                .count();

            let sync_count = lines
                .iter()
                .filter(|l| {
                    (l.contains("fn ") || l.contains("def "))
                        && !l.contains("async fn ")
                        && !l.contains("async def ")
                })
                .count();

            file_details.push(serde_json::json!({
                "path": fp,
                "total_lines": total_lines,
                "class_names": class_names,
                "async_functions": async_count,
                "sync_functions": sync_count,
            }));
        }

        let total_classes: usize = file_details
            .iter()
            .map(|f| {
                f.get("class_names")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0)
            })
            .sum();

        let total_functions: usize = file_details
            .iter()
            .map(|f| {
                let a = f
                    .get("async_functions")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let s = f
                    .get("sync_functions")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                (a + s) as usize
            })
            .sum();

        // ─── LLM Enhancement (if available) ──────────────
        let mut llm_analysis: Option<serde_json::Value> = None;
        if let (Some(llm), Some(model)) = (llm, model) {
            // Pick top 2 files for LLM analysis (keep it under ~30s)
            let top_files: Vec<_> = file_details.iter().take(2).collect();
            let mut file_summaries = Vec::new();

            for fdata in &top_files {
                if let Some(fp) = fdata.get("path").and_then(|v| v.as_str()) {
                    // Read more content for LLM — limit to 1500 chars
                    let content_result = registry
                        .execute(
                            "read_file",
                            &HashMap::from([
                                ("path".into(), serde_json::json!(fp)),
                                ("limit".into(), serde_json::json!(300)),
                            ]),
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
                            let truncated = if content.len() > 1500 {
                                let end = content
                                    .char_indices()
                                    .take_while(|(idx, _)| *idx < 1500)
                                    .last()
                                    .map(|(idx, c)| idx + c.len_utf8())
                                    .unwrap_or(content.len());
                                &content[..end]
                            } else {
                                content
                            };

                            let prompt = format!(
                                "Briefly describe this file's purpose and key patterns (max 3 sentences).\n\
                                 File: {}\n\n{}", fp, truncated
                            );

                            match llm
                                .chat(vec![ChatMessage::user(prompt)], model, 0.1, 256)
                                .await
                            {
                                Ok(resp) => {
                                    file_summaries.push(serde_json::json!({
                                        "path": fp,
                                        "llm_analysis": resp.content,
                                    }));
                                }
                                Err(e) => {
                                    tracing::warn!("LLM analysis failed for {}: {}", fp, e);
                                }
                            }
                        }
                    }
                }
            }

            if !file_summaries.is_empty() {
                llm_analysis = Some(serde_json::json!({
                    "llm_file_analyses": file_summaries,
                    "method": "llm_enhanced",
                }));
            }
        }

        let final_results = if let Some(llm) = llm_analysis {
            serde_json::json!({
                "task": task,
                "total_files": total_files,
                "files_analysed": file_details.len(),
                "files": file_details,
                "total_classes": total_classes,
                "total_functions": total_functions,
                "llm_analysis": llm,
            })
        } else {
            serde_json::json!({
                "task": task,
                "total_files": total_files,
                "files_analysed": file_details.len(),
                "files": file_details,
                "total_classes": total_classes,
                "total_functions": total_functions,
            })
        };

        self.impl_
            .write_memory(memory, "researcher_map", &final_results, None);
        self.impl_.cleanup().await;

        AgentResult::ok(final_results)
    }
}
