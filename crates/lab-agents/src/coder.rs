//! CoderAgent — generates/edits code files in the workspace.

use crate::base::AgentImpl;
use lab_core::config::AgentProfile;
use lab_core::llm::LLMClient;
use lab_core::types::AgentResult;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct CoderAgent {
    impl_: AgentImpl,
    pub workspace: PathBuf,
}

impl CoderAgent {
    pub fn new(id: String, session_id: String, profile: AgentProfile, workspace: PathBuf) -> Self {
        Self {
            impl_: AgentImpl::new(
                id.clone(), "coder".into(), session_id, profile, workspace.clone(),
            ),
            workspace,
        }
    }

    pub fn id(&self) -> &str { self.impl_.id() }
    pub fn session_id(&self) -> &str { self.impl_.session_id() }
    pub fn state(&self) -> lab_core::types::AgentState { self.impl_.state() }

    /// Execute: create or write a file. Supports direct content or template generation.
    pub async fn execute(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        task: &str,
        path: Option<&str>,
        content: Option<&str>,
        template: Option<&str>,
        _llm: Option<&dyn lab_core::llm::LLMClient>,
        _model: Option<&str>,
    ) -> AgentResult {
        if let Err(e) = self.impl_.start().await {
            return AgentResult::fail(e.to_string(), None);
        }

        let mut results = HashMap::new();
        let mut written_files = Vec::new();

        if let Some(p) = path {
            let text = content.unwrap_or("");
            if !text.is_empty() {
                // Direct write mode
                let result = registry.execute("write_file", &HashMap::from([
                    ("path".into(), serde_json::json!(p)),
                    ("content".into(), serde_json::json!(text)),
                ])).await;

                if !result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                    self.impl_.cleanup().await;
                    return AgentResult::fail(format!("Failed to write {p}"), Some(result));
                }

                let bytes = result.get("data").and_then(|d| d.get("bytes_written")).and_then(|v| v.as_u64()).unwrap_or(0);
                written_files.push(serde_json::json!({"path": p, "bytes": bytes}));
            }

            // Verify what we wrote
            let verify = registry.execute("read_file", &HashMap::from([
                ("path".into(), serde_json::json!(p)),
                ("limit".into(), serde_json::json!(5)),
            ])).await;

            results.insert("verified".to_string(), serde_json::json!(
                verify.get("success").and_then(|v| v.as_bool()).unwrap_or(false)
            ));
            if let Some(preview) = verify.get("data").and_then(|d| d.get("content")).and_then(|c| c.as_str()) {
                results.insert("preview".to_string(), serde_json::json!(&preview[..preview.len().min(500)]));
            }
        } else if let Some(tmpl) = template {
            // Template generation mode
            let name = tmpl.replace('_', " ").replace('-', " ");
            let output_path = format!("src/{name}.py");
            let template_content = format!("\"\"\"{name} module.\"\"\"\n\nfrom __future__ import annotations\n\nimport logging\n\nlogger = logging.getLogger(__name__)\n\n\ndef main() -> None:\n    \"\"\"Entry point for {name}.\"\"\"\n    logger.info(\"{name} running\")\n\n\nif __name__ == \"__main__\":\n    main()\n");

            let result = registry.execute("write_file", &HashMap::from([
                ("path".into(), serde_json::json!(output_path)),
                ("content".into(), serde_json::json!(template_content)),
            ])).await;

            if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                let bytes = result.get("data").and_then(|d| d.get("bytes_written")).and_then(|v| v.as_u64()).unwrap_or(0);
                written_files.push(serde_json::json!({"path": output_path, "bytes": bytes}));
            } else {
                self.impl_.cleanup().await;
                return AgentResult::fail(format!("Failed to generate template for {name}"), Some(result));
            }
        }

        results.insert("task".into(), serde_json::json!(task));
        results.insert("written_files".into(), serde_json::json!(written_files));
        let data = serde_json::json!(results);

        self.impl_.write_memory(memory, "coder_output", &data, None);
        self.impl_.cleanup().await;

        AgentResult::ok(data)
    }
}
