//! ReviewerAgent — static analysis and code quality review.
//!
//! For **Rust workspaces** (Cargo.toml present): runs `cargo clippy
//! --message-format json` via the `improvement/clippy_runner` and maps
//! each `ClippyFinding` into the standard `reviews` output shape.
//!
//! For **everything else**: falls back to the original regex / text-scan
//! path so Python, JavaScript, etc. still get reviewed.
//!
//! Either way the output JSON is identical:
//! ```json
//! {
//!   "reviews": [{ "path", "lines", "issues": [{"rule","line","msg"}], "issue_count" }],
//!   "summary": { "files_reviewed", "total_issues", "by_rule": {rule: count} }
//! }
//! ```

use crate::base::AgentImpl;
use crate::improvement::{run_clippy, ClippyFinding};
use lab_core::config::AgentProfile;
use lab_core::llm::ChatMessage;
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
                id.clone(),
                "reviewer".into(),
                session_id,
                profile,
                workspace.clone(),
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
        llm: Option<&dyn lab_core::llm::LLMClient>,
        model: Option<&str>,
    ) -> AgentResult {
        if let Err(e) = self.impl_.start().await {
            return AgentResult::fail(e.to_string(), None);
        }

        let is_rust = self.workspace.join("Cargo.toml").exists();
        let reviews = if is_rust {
            self.clippy_review(pattern, path, file_limit).await
        } else {
            self.regex_review(registry, pattern, path, file_limit).await
        };

        // ─── Summary ───────────────────────────────────────────────
        let mut by_rule: HashMap<String, usize> = HashMap::new();
        let mut total_issues = 0usize;
        for review in &reviews {
            let count = review
                .get("issue_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            total_issues += count;
            if let Some(issues) = review.get("issues").and_then(|v| v.as_array()) {
                for issue in issues {
                    let rule = issue
                        .get("rule")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    *by_rule.entry(rule).or_insert(0) += 1;
                }
            }
        }
        let files_reviewed = reviews.len();

        // ─── LLM deep review for top files ─────────────────────────
        let llm_insights = if let (Some(llm), Some(model)) = (llm, model) {
            self.llm_deep_review(registry, &reviews, llm, model).await
        } else {
            Vec::new()
        };

        let data = if llm_insights.is_empty() {
            serde_json::json!({
                "task": task,
                "backend": if is_rust { "clippy" } else { "regex" },
                "reviews": reviews,
                "summary": {
                    "files_reviewed": files_reviewed,
                    "total_issues": total_issues,
                    "by_rule": by_rule,
                }
            })
        } else {
            serde_json::json!({
                "task": task,
                "backend": if is_rust { "clippy" } else { "regex" },
                "reviews": reviews,
                "llm_insights": llm_insights,
                "llm_enhanced": true,
                "summary": {
                    "files_reviewed": files_reviewed,
                    "total_issues": total_issues,
                    "by_rule": by_rule,
                }
            })
        };

        self.impl_.write_memory(memory, "review_results", &data, None);
        self.impl_.cleanup().await;
        AgentResult::ok(data)
    }

    // ─── Clippy-backed review (Rust workspaces) ─────────────────────

    async fn clippy_review(
        &self,
        pattern: Option<&str>,
        path: Option<&str>,
        file_limit: Option<usize>,
    ) -> Vec<serde_json::Value> {
        let findings: Vec<ClippyFinding> = run_clippy(&self.workspace).await;

        // Group findings by file
        let mut by_file: HashMap<PathBuf, Vec<&ClippyFinding>> = HashMap::new();
        for finding in &findings {
            by_file.entry(finding.file.clone()).or_default().push(finding);
        }

        // Extract a simple substring hint from the pattern (e.g. "src/" from "src/**/*.rs")
        // and from the path filter, to narrow down which files to include.
        let path_filter = path.map(|p| p.to_lowercase());
        let pattern_hint: Option<String> = pattern.and_then(|p| {
            // If the pattern contains a directory prefix before **, use it as a filter.
            let before_star = p.split('*').next().unwrap_or("").trim_end_matches('/');
            if before_star.is_empty() { None } else { Some(before_star.to_lowercase()) }
        });

        let limit = file_limit.unwrap_or(50).min(200);

        let mut reviews: Vec<serde_json::Value> = by_file
            .iter()
            .filter(|(file, _)| {
                let s = file.to_string_lossy().to_lowercase();
                if let Some(ref pf) = path_filter {
                    if !s.contains(pf.as_str()) {
                        return false;
                    }
                }
                if let Some(ref hint) = pattern_hint {
                    if !s.contains(hint.as_str()) {
                        return false;
                    }
                }
                true
            })
            .map(|(file, file_findings)| {
                let issues: Vec<serde_json::Value> = file_findings
                    .iter()
                    .map(|f| serde_json::json!({
                        "rule": f.lint,
                        "line": f.line_start,
                        "msg":  f.message,
                        "applicability": format!("{:?}", f.applicability),
                    }))
                    .collect();
                let count = issues.len();
                serde_json::json!({
                    "path":        file.display().to_string(),
                    "lines":       0,   // not available without re-reading the file
                    "issues":      issues,
                    "issue_count": count,
                })
            })
            .collect();

        // Sort by issue count descending so worst files appear first
        reviews.sort_by(|a, b| {
            let ca = a.get("issue_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let cb = b.get("issue_count").and_then(|v| v.as_u64()).unwrap_or(0);
            cb.cmp(&ca)
        });
        reviews.truncate(limit);
        reviews
    }

    // ─── Regex/text review (non-Rust workspaces) ────────────────────

    async fn regex_review(
        &self,
        registry: &mut ToolRegistry,
        pattern: Option<&str>,
        path: Option<&str>,
        file_limit: Option<usize>,
    ) -> Vec<serde_json::Value> {
        let pattern = pattern.unwrap_or("**/*.py");

        let files_result = registry
            .execute(
                "glob_search",
                &HashMap::from([("pattern".into(), serde_json::json!(pattern))]),
            )
            .await;

        if !files_result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Vec::new();
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
        let bare_except_re = Regex::new(r"^\s*except\s*:").ok();
        let bare_catch_re = Regex::new(r"^\s*\.unwrap\(\)").ok();
        let mut reviews = Vec::new();

        for fp in file_list.iter().take(limit) {
            let content_result = registry
                .execute(
                    "read_file",
                    &HashMap::from([("path".into(), serde_json::json!(fp))]),
                )
                .await;

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

            let has_docstring = lines.iter().take(10).any(|l| {
                l.contains("\"\"\"")
                    || l.contains("'''")
                    || l.contains("//!")
                    || l.contains("///")
            });
            if !has_docstring {
                issues.push(serde_json::json!({ "rule": "missing_docstring", "line": 1,
                    "msg": "No module docstring" }));
            }

            for (i, line) in lines.iter().enumerate() {
                if line.len() > 120 {
                    issues.push(serde_json::json!({ "rule": "line_too_long", "line": i + 1,
                        "msg": format!("Line length {} exceeds 120", line.len()) }));
                }
            }

            if let Some(ref re) = bare_except_re {
                for (i, line) in lines.iter().enumerate() {
                    if re.is_match(line) {
                        issues.push(serde_json::json!({ "rule": "bare_except", "line": i + 1,
                            "msg": "Use 'except Exception:' instead of bare except" }));
                    }
                }
            }

            if let Some(ref re) = bare_catch_re {
                for (i, line) in lines.iter().enumerate() {
                    if re.is_match(line) {
                        issues.push(serde_json::json!({ "rule": "unsafe_unwrap", "line": i + 1,
                            "msg": "Consider using ? or match instead of unwrap()" }));
                    }
                }
            }

            for (i, line) in lines.iter().enumerate() {
                let upper = line.to_uppercase();
                for tag in &["TODO", "FIXME", "HACK"] {
                    if upper.contains(*tag) {
                        issues.push(serde_json::json!({ "rule": format!("pending_{}", tag.to_lowercase()),
                            "line": i + 1, "msg": format!("{} comment found", tag) }));
                    }
                }
            }

            let count = issues.len();
            reviews.push(serde_json::json!({
                "path": fp,
                "lines": lines.len(),
                "issues": issues,
                "issue_count": count,
            }));
        }

        reviews
    }

    // ─── LLM deep review of top N files ─────────────────────────────

    async fn llm_deep_review(
        &self,
        registry: &mut ToolRegistry,
        reviews: &[serde_json::Value],
        llm: &dyn lab_core::llm::LLMClient,
        model: &str,
    ) -> Vec<serde_json::Value> {
        let system = "\
You are a rigorous code reviewer for the AI Research Lab — a Rust/Python \
multi-agent workspace. Identify concrete, actionable quality issues. \
Focus on: error handling (unwrap/panic risks), API surface design, \
performance anti-patterns, missing documentation, and security concerns. \
Format your response as a numbered list. Be specific — cite line patterns \
or function names. Do not praise the code; only report issues and improvements.";

        let mut insights = Vec::new();
        for review in reviews.iter().take(2) {
            let Some(fp) = review.get("path").and_then(|v| v.as_str()) else { continue };

            let cr = registry
                .execute(
                    "read_file",
                    &HashMap::from([
                        ("path".into(), serde_json::json!(fp)),
                        ("limit".into(), serde_json::json!(150)),
                    ]),
                )
                .await;

            let Some(content) = cr
                .get("data")
                .and_then(|d| d.get("content"))
                .and_then(|c| c.as_str())
            else {
                continue;
            };

            let snippet = if content.len() > 2500 { &content[..2500] } else { content };
            let prompt = format!("File: `{fp}`\n\n```\n{snippet}\n```");

            match llm
                .chat(vec![ChatMessage::system(system), ChatMessage::user(prompt)], model, 0.1, 512)
                .await
            {
                Ok(resp) => insights.push(serde_json::json!({ "path": fp, "review": resp.content })),
                Err(e) => tracing::warn!("LLM review for {}: {}", fp, e),
            }
        }
        insights
    }
}
