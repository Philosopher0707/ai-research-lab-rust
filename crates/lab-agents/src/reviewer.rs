//! ReviewerAgent — static analysis and code quality review.

use crate::base::AgentImpl;
use lab_core::config::AgentProfile;
use lab_core::llm::LLMClient;
use lab_core::types::AgentResult;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ReviewerAgent {
    impl_: AgentImpl,
    pub workspace: PathBuf,
}

impl ReviewerAgent {
    pub fn new(id: String, session_id: String, profile: AgentProfile, workspace: PathBuf) -> Self {
        Self {
            impl_: AgentImpl::new(
                id.clone(), "reviewer".into(), session_id, profile, workspace.clone(),
            ),
            workspace,
        }
    }

    pub fn id(&self) -> &str { self.impl_.id() }
    pub fn session_id(&self) -> &str { self.impl_.session_id() }
    pub fn state(&self) -> lab_core::types::AgentState { self.impl_.state() }

    /// Execute: review files for code quality issues.
    pub async fn execute(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        task: &str,
        pattern: Option<&str>,
        path: Option<&str>,
        file_limit: Option<usize>,
        _llm: Option<&dyn lab_core::llm::LLMClient>,
        _model: Option<&str>,
    ) -> AgentResult {
        if let Err(e) = self.impl_.start().await {
            return AgentResult::fail(e.to_string(), None);
        }

        let pattern = pattern.unwrap_or("**/*.py");

        // 1. Discover files
        let files_result = registry.execute("glob_search", &HashMap::from([
            ("pattern".into(), serde_json::json!(pattern)),
        ])).await;

        if !files_result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            self.impl_.cleanup().await;
            return AgentResult::fail("File discovery failed", Some(files_result));
        }

        let mut file_list: Vec<String> = files_result
            .get("data")
            .and_then(|d| d.get("matches"))
            .and_then(|m| m.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        if let Some(p) = path {
            let p_lower = p.to_lowercase();
            file_list.retain(|f| f.to_lowercase().contains(&p_lower));
        }

        let limit = file_limit.unwrap_or(30).min(100);
        let mut reviews = Vec::new();

        // Bare except regex
        let bare_except_re = Regex::new(r"^\s*except\s*:").ok();
        // Bare except Rust
        let bare_catch_re = Regex::new(r"^\s*\.unwrap\(\)").ok();

        for fp in file_list.iter().take(limit) {
            let content_result = registry.execute("read_file", &HashMap::from([
                ("path".into(), serde_json::json!(fp)),
            ])).await;

            if !content_result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }

            let content = content_result
                .get("data")
                .and_then(|d| d.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("");

            let lines: Vec<&str> = content.lines().collect();
            let mut issues = Vec::new();

            // Check docstrings (first 10 lines)
            let has_docstring = lines.iter().take(10).any(|l| l.contains("\"\"\"") || l.contains("'''") || l.contains("//!") || l.contains("///"));
            if !has_docstring {
                issues.push(serde_json::json!({
                    "rule": "missing_docstring",
                    "line": 1,
                    "msg": "No module docstring"
                }));
            }

            // Check line lengths
            for (i, line) in lines.iter().enumerate() {
                if line.len() > 120 {
                    issues.push(serde_json::json!({
                        "rule": "line_too_long",
                        "line": i + 1,
                        "msg": format!("Line length {} exceeds 120", line.len()),
                    }));
                }
            }

            // Check bare except
            if let Some(ref re) = bare_except_re {
                for (i, line) in lines.iter().enumerate() {
                    if re.is_match(line) {
                        issues.push(serde_json::json!({
                            "rule": "bare_except",
                            "line": i + 1,
                            "msg": "Use 'except Exception:' instead of bare except",
                        }));
                    }
                }
            }

            // Check unwrap() calls (Rust anti-pattern when Result could be Err)
            if let Some(ref re) = bare_catch_re {
                for (i, line) in lines.iter().enumerate() {
                    if re.is_match(line) {
                        issues.push(serde_json::json!({
                            "rule": "unsafe_unwrap",
                            "line": i + 1,
                            "msg": "Consider using ? or match instead of unwrap()",
                        }));
                    }
                }
            }

            // Check TODO/FIXME/HACK comments
            for (i, line) in lines.iter().enumerate() {
                let upper = line.to_uppercase();
                for tag in &["TODO", "FIXME", "HACK"] {
                    if upper.contains(*tag) {
                        issues.push(serde_json::json!({
                            "rule": format!("pending_{}", tag.to_lowercase()),
                            "line": i + 1,
                            "msg": format!("{} comment found", tag),
                        }));
                    }
                }
            }

            reviews.push(serde_json::json!({
                "path": fp,
                "lines": lines.len(),
                "issues": issues,
                "issue_count": issues.len(),
            }));
        }

        // Summary
        let mut by_rule = HashMap::new();
        let mut total_issues = 0;
        for review in &reviews {
            total_issues += review.get("issue_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if let Some(issues) = review.get("issues").and_then(|v| v.as_array()) {
                for issue in issues {
                    let rule = issue.get("rule").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    *by_rule.entry(rule).or_insert(0) += 1;
                }
            }
        }

        let data = serde_json::json!({
            "task": task,
            "reviews": reviews,
            "summary": {
                "files_reviewed": reviews.len(),
                "total_issues": total_issues,
                "by_rule": by_rule,
            }
        });

        self.impl_.write_memory(memory, "review_results", &data, None);
        self.impl_.cleanup().await;
        AgentResult::ok(data)
    }
}
