//! SelfEditAgent — LLM-driven targeted file editing with build verification.
//!
//! Loop:
//!   1. Read the target file
//!   2. Ask LLM to output <edits>[{old, new}, ...]</edits>
//!   3. Apply edits via edit_file tool
//!   4. Verify with `cargo check` (via bash tool)
//!   5. Revert via `git checkout` if verification fails

use crate::base::AgentImpl;
use lab_core::config::AgentProfile;
use lab_core::llm::{ChatMessage, LLMClient};
use lab_core::types::AgentResult;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct SelfEditAgent {
    impl_: AgentImpl,
    pub workspace: PathBuf,
}

impl SelfEditAgent {
    pub fn new(id: String, session_id: String, profile: AgentProfile, workspace: PathBuf) -> Self {
        Self {
            impl_: AgentImpl::new(
                id.clone(),
                "self_edit".into(),
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

    /// Run the self-edit cycle on a single file.
    ///
    /// - `file_path` — path to the file (relative to workspace or absolute)
    /// - `task`      — natural-language description of the improvement
    /// - `dry_run`   — if true, parse and return proposed edits without applying them
    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        file_path: &str,
        task: &str,
        dry_run: bool,
        llm: &dyn LLMClient,
        model: &str,
    ) -> AgentResult {
        if let Err(e) = self.impl_.start().await {
            return AgentResult::fail(e.to_string(), None);
        }

        // ── 1. Read the file ──────────────────────────────────────
        let read_result = registry
            .execute(
                "read_file",
                &HashMap::from([("path".into(), serde_json::json!(file_path))]),
            )
            .await;

        if !read_result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            self.impl_.cleanup().await;
            let err = read_result
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("read failed")
                .to_string();
            return AgentResult::fail(format!("Cannot read {file_path}: {err}"), None);
        }

        let content = read_result
            .get("data")
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let total_lines = read_result
            .get("data")
            .and_then(|d| d.get("total_lines"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Truncate very large files to stay within LLM context
        let snippet = if content.len() > 8000 {
            format!(
                "{}\n\n// [truncated — {} total lines]",
                &content[..8000],
                total_lines
            )
        } else {
            content.clone()
        };

        // ── 2. Ask LLM for edits ──────────────────────────────────
        let system = "\
You are a Rust/Python refactoring engineer for the AI Research Lab codebase. \
Make targeted, minimal improvements to the given file based on the stated task.\n\
\n\
Output your changes as a JSON array wrapped in <edits></edits> tags.\n\
Each edit: {\"old\": \"exact original text\", \"new\": \"replacement text\"}\n\
\n\
Rules:\n\
- \"old\" must be EXACT bytes from the file — same whitespace, same indentation\n\
- Prefer several small focused edits over one large rewrite\n\
- Do NOT touch unrelated code\n\
- If no change is needed: <edits>[]</edits>\n\
- Output ONLY the <edits>...</edits> block — no explanation, no markdown fences";

        let prompt = format!("File: {file_path}\nTask: {task}\n\n```\n{snippet}\n```");

        let llm_response = match llm
            .chat(
                vec![ChatMessage::system(system), ChatMessage::user(prompt)],
                model,
                0.1,
                2048,
            )
            .await
        {
            Ok(r) => r.content,
            Err(e) => {
                self.impl_.cleanup().await;
                return AgentResult::fail(format!("LLM error: {e}"), None);
            }
        };

        // ── 3. Parse edits ────────────────────────────────────────
        let edits = parse_edits(&llm_response);

        if edits.is_empty() {
            self.impl_.cleanup().await;
            let result = serde_json::json!({
                "file": file_path,
                "task": task,
                "edits_proposed": 0,
                "edits_applied": 0,
                "message": "LLM proposed no changes",
            });
            self.impl_
                .write_memory(memory, "self_edit_result", &result, None);
            return AgentResult::ok(result);
        }

        // ── Dry run — return without touching the file ────────────
        if dry_run {
            self.impl_.cleanup().await;
            let result = serde_json::json!({
                "file": file_path,
                "task": task,
                "dry_run": true,
                "edits_proposed": edits.len(),
                "edits": edits,
            });
            self.impl_
                .write_memory(memory, "self_edit_result", &result, None);
            return AgentResult::ok(result);
        }

        // ── 4. Apply edits ────────────────────────────────────────
        let mut applied = 0usize;
        let mut apply_errors: Vec<String> = Vec::new();

        for edit in &edits {
            let old = edit.get("old").and_then(|v| v.as_str()).unwrap_or("");
            let new = edit.get("new").and_then(|v| v.as_str()).unwrap_or("");
            if old.is_empty() {
                continue;
            }

            let r = registry
                .execute(
                    "edit_file",
                    &HashMap::from([
                        ("path".into(), serde_json::json!(file_path)),
                        ("old_string".into(), serde_json::json!(old)),
                        ("new_string".into(), serde_json::json!(new)),
                    ]),
                )
                .await;

            if r.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                applied += 1;
            } else {
                let err = r
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("edit failed")
                    .to_string();
                apply_errors.push(err);
            }
        }

        if applied == 0 {
            self.impl_.cleanup().await;
            return AgentResult::fail(
                format!(
                    "No edits applied ({} errors). Check that 'old' strings match exactly.",
                    apply_errors.len()
                ),
                Some(serde_json::json!({
                    "file": file_path,
                    "edits_proposed": edits.len(),
                    "errors": apply_errors,
                    "proposed_edits": edits,
                })),
            );
        }

        // ── 5. Verify with cargo check ────────────────────────────
        let ws = shell_escape(self.workspace.to_string_lossy().as_ref());
        let check_cmd = format!("cd {ws} && cargo check --message-format short 2>&1 | head -60");

        let check_result = registry
            .execute(
                "bash",
                &HashMap::from([("command".into(), serde_json::json!(check_cmd))]),
            )
            .await;

        let exit_code = check_result
            .get("data")
            .and_then(|d| d.get("returncode"))
            .and_then(|v| v.as_i64())
            .unwrap_or(1);

        let compiler_out = check_result
            .get("data")
            .and_then(|d| d.get("stdout").or_else(|| d.get("output")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if exit_code != 0 {
            // ── 6. Revert on failure ──────────────────────────────
            let fp = shell_escape(file_path);
            let revert_cmd = format!("git -C {ws} checkout -- {fp}");
            registry
                .execute(
                    "bash",
                    &HashMap::from([("command".into(), serde_json::json!(revert_cmd))]),
                )
                .await;

            self.impl_.cleanup().await;
            return AgentResult::fail(
                format!("cargo check failed — reverted {file_path}"),
                Some(serde_json::json!({
                    "file": file_path,
                    "edits_applied": applied,
                    "reverted": true,
                    "compiler_errors": compiler_out,
                    "proposed_edits": edits,
                })),
            );
        }

        // ── 7. Success ────────────────────────────────────────────
        let result = serde_json::json!({
            "file": file_path,
            "task": task,
            "edits_proposed": edits.len(),
            "edits_applied": applied,
            "apply_errors": apply_errors,
            "verified": true,
            "compiler_output": compiler_out,
            "edits": edits,
        });

        self.impl_
            .write_memory(memory, "self_edit_result", &result, None);
        self.impl_.cleanup().await;
        AgentResult::ok(result)
    }
}

// ─── Edit Parsing ─────────────────────────────────────────────────

fn parse_edits(text: &str) -> Vec<serde_json::Value> {
    let open = "<edits>";
    let close = "</edits>";
    let start = match text.find(open) {
        Some(i) => i + open.len(),
        None => return Vec::new(),
    };
    let end = match text[start..].find(close) {
        Some(i) => start + i,
        None => return Vec::new(),
    };
    serde_json::from_str::<Vec<serde_json::Value>>(text[start..end].trim()).unwrap_or_default()
}

// ─── Shell Escaping ───────────────────────────────────────────────

/// Wrap a path in single quotes for POSIX shell, escaping any ' inside.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
