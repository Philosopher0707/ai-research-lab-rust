//! SummarizerAgent — reads memory and generates consolidated reports.

use crate::base::AgentImpl;
use lab_core::config::AgentProfile;
use lab_core::llm::ChatMessage;
use lab_core::types::AgentResult;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct SummarizerAgent {
    pub impl_: AgentImpl,
    pub workspace: PathBuf,
}

impl SummarizerAgent {
    pub fn new(id: String, session_id: String, profile: AgentProfile, workspace: PathBuf) -> Self {
        Self {
            impl_: AgentImpl::new(
                id.clone(),
                "summarizer".into(),
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
    pub fn state(&self) -> lab_core::types::AgentState {
        self.impl_.state()
    }

    /// Execute: pull data from memory and generate a consolidated markdown report.
    pub async fn execute(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        task: &str,
        output_path: Option<&str>,
        llm: Option<&dyn lab_core::llm::LLMClient>,
        model: Option<&str>,
    ) -> AgentResult {
        if let Err(e) = self.impl_.start().await {
            return AgentResult::fail(e.to_string(), None);
        }

        let mut report = String::new();
        report.push_str("# AI Research Lab — Session Summary\n\n");
        report.push_str(&format!("**Session:** {}\n\n", self.impl_.session_id()));
        report.push_str(&format!("**Agent:** {}\n\n", self.impl_.id()));

        // Pull researcher data
        if let Some(research) = self.impl_.read_memory(memory, "researcher_map") {
            report.push_str("## Codebase Structure\n\n");
            if let Some(tf) = research.get("total_files").and_then(|v| v.as_u64()) {
                report.push_str(&format!("- Files analysed: {tf}\n"));
            }
            if let Some(tc) = research.get("total_classes").and_then(|v| v.as_u64()) {
                report.push_str(&format!("- Classes found: {tc}\n"));
            }
            if let Some(tfu) = research.get("total_functions").and_then(|v| v.as_u64()) {
                report.push_str(&format!("- Functions found: {tfu}\n"));
            }
            report.push('\n');
        }

        // Pull review data
        if let Some(reviews) = self.impl_.read_memory(memory, "review_results") {
            report.push_str("## Code Quality Review\n\n");
            if let Some(summary) = reviews.get("summary") {
                if let Some(fr) = summary.get("files_reviewed").and_then(|v| v.as_u64()) {
                    report.push_str(&format!("- Files reviewed: {fr}\n"));
                }
                if let Some(ti) = summary.get("total_issues").and_then(|v| v.as_u64()) {
                    report.push_str(&format!("- Total issues: {ti}\n"));
                }
                if let Some(by_rule) = summary.get("by_rule").and_then(|v| v.as_object()) {
                    for (rule, count) in by_rule {
                        report.push_str(&format!("  - `{rule}`: {count}\n"));
                    }
                }
            }
            report.push('\n');
        }

        // Pull coder data
        if let Some(coder_out) = self.impl_.read_memory(memory, "coder_output") {
            report.push_str("## Code Generation\n\n");
            if let Some(files) = coder_out.get("written_files").and_then(|v| v.as_array()) {
                for f in files {
                    if let Some(p) = f.get("path").and_then(|v| v.as_str()) {
                        let b = f.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                        report.push_str(&format!("- `{p}` ({b} bytes)\n"));
                    }
                }
            }
            report.push('\n');
        }

        // List memory keys
        let keys = self.impl_.list_memory_keys(memory, None);
        report.push_str(&format!("## Memory\n\n- Keys: {}\n", keys.join(", ")));

        // LLM synthesis — append AI insights to the report
        if let (Some(llm_client), Some(model)) = (llm, model) {
            let context: String = report.chars().take(3000).collect();
            if !context.is_empty() {
                let system = "\
You are the synthesis agent for the AI Research Lab. Your role is to read structured research \
and review data produced by other agents and distill it into executive-level insights. \
Output exactly 3–5 bullet-point insights, followed by a 'Recommended Actions' section with \
3–5 concrete next steps. Be specific and actionable — no vague suggestions. \
Write in a technical tone suitable for a senior engineering team.";
                let prompt = format!("Session data:\n\n{context}");
                match llm_client
                    .chat(
                        vec![ChatMessage::system(system), ChatMessage::user(prompt)],
                        model,
                        0.2,
                        512,
                    )
                    .await
                {
                    Ok(resp) => {
                        report.push_str("\n## AI Insights\n\n");
                        report.push_str(&resp.content);
                        report.push('\n');
                    }
                    Err(e) => tracing::warn!("LLM synthesis failed: {}", e),
                }
            }
        }

        // Write report to disk
        let out = output_path.unwrap_or("lab-outputs/summary.md");
        let write_result = registry
            .execute(
                "write_file",
                &HashMap::from([
                    ("path".into(), serde_json::json!(out)),
                    ("content".into(), serde_json::json!(&report)),
                ]),
            )
            .await;

        let success = write_result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let results = serde_json::json!({
            "task": task,
            "summary_written": success,
            "output_path": out,
            "report_length": report.len(),
        });

        self.impl_.write_memory(memory, "summary", &results, None);
        self.impl_.cleanup().await;

        AgentResult::ok(results)
    }
}
