//! Edit strategies for applying clippy findings to source code.
//!
//! Three implementations, dispatched by `RiskLevel`:
//!
//! | Strategy          | How                                              |
//! |-------------------|--------------------------------------------------|
//! | `ClippyFixStrategy` | `cargo clippy --fix -- -A clippy::all -W <lint>` |
//! | `DiffPatchStrategy` | LLM outputs unified diff → applied via `diffy`   |
//! | `LLMEditStrategy`   | LLM replaces compiler-provided byte range         |

use super::clippy_runner::ClippyFinding;
use async_trait::async_trait;
use lab_core::llm::{ChatMessage, LLMClient};
use std::path::{Path, PathBuf};

// ─── EditResult ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EditResult {
    pub success: bool,
    pub file: PathBuf,
    /// Human-readable description of what happened (or the error).
    pub detail: String,
}

impl EditResult {
    fn ok(file: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        Self {
            success: true,
            file: file.into(),
            detail: detail.into(),
        }
    }
    fn err(file: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        Self {
            success: false,
            file: file.into(),
            detail: detail.into(),
        }
    }
}

// ─── EditStrategy trait ─────────────────────────────────────────────

#[async_trait]
pub trait EditStrategy: Send + Sync {
    /// Apply the edit for `finding` in `workspace`. Returns the result.
    async fn apply(&self, finding: &ClippyFinding, workspace: &Path) -> EditResult;
}

// ─── ClippyFixStrategy ──────────────────────────────────────────────

/// Runs `cargo clippy --fix` scoped to the exact lint that triggered the finding.
/// This is completely LLM-free — the compiler generates and applies its own suggestion.
pub struct ClippyFixStrategy;

#[async_trait]
impl EditStrategy for ClippyFixStrategy {
    async fn apply(&self, finding: &ClippyFinding, workspace: &Path) -> EditResult {
        // e.g. "clippy::derivable_impls" → "-A clippy::all -W clippy::derivable_impls"
        let lint_flag = format!("-A clippy::all -W {}", finding.lint);

        let output = tokio::process::Command::new("cargo")
            .args([
                "clippy",
                "--fix",
                "--allow-dirty",
                "--allow-staged",
                "--",
                "-A",
                "clippy::all",
                "-W",
                &finding.lint,
            ])
            .current_dir(workspace)
            .output()
            .await;

        let file = &finding.file;
        match output {
            Ok(o) if o.status.success() => {
                EditResult::ok(file, format!("cargo clippy --fix applied ({lint_flag})"))
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
                EditResult::err(file, format!("cargo clippy --fix failed: {stderr}"))
            }
            Err(e) => EditResult::err(file, format!("failed to spawn cargo: {e}")),
        }
    }
}

// ─── DiffPatchStrategy ──────────────────────────────────────────────

/// Asks the LLM for a unified diff patch, then applies it with `diffy`.
pub struct DiffPatchStrategy {
    pub llm: Box<dyn LLMClient>,
    pub model: String,
}

#[async_trait]
impl EditStrategy for DiffPatchStrategy {
    async fn apply(&self, finding: &ClippyFinding, workspace: &Path) -> EditResult {
        let file_path = workspace.join(&finding.file);
        let source = match tokio::fs::read_to_string(&file_path).await {
            Ok(s) => s,
            Err(e) => return EditResult::err(&finding.file, format!("read error: {e}")),
        };

        let prompt = build_diff_prompt(finding, &source);
        let messages = vec![
            ChatMessage::system(DIFF_SYSTEM_PROMPT),
            ChatMessage::user(prompt),
        ];

        let response = match self.llm.chat(messages, &self.model, 0.1, 2048).await {
            Ok(r) => r.content,
            Err(e) => return EditResult::err(&finding.file, format!("LLM error: {e}")),
        };

        let patch = extract_diff(&response);
        if patch.is_empty() {
            return EditResult::err(&finding.file, "LLM returned no unified diff".to_string());
        }

        match diffy::apply(
            &source,
            &diffy::Patch::from_str(&patch).unwrap_or_else(|_| diffy::Patch::from_str("").unwrap()),
        ) {
            Ok(patched) => {
                if let Err(e) = tokio::fs::write(&file_path, patched).await {
                    return EditResult::err(&finding.file, format!("write error: {e}"));
                }
                EditResult::ok(&finding.file, "diff patch applied")
            }
            Err(e) => EditResult::err(&finding.file, format!("patch apply failed: {e}")),
        }
    }
}

const DIFF_SYSTEM_PROMPT: &str = "\
You are a Rust expert fixing clippy warnings. \
Output ONLY a unified diff (--- a/file / +++ b/file) that fixes the warning. \
No explanation, no markdown fences — just the raw diff.";

fn build_diff_prompt(finding: &ClippyFinding, source: &str) -> String {
    let context_start = finding.line_start.saturating_sub(5) as usize;
    let context_end = (finding.line_end as usize + 5).min(source.lines().count());
    let snippet: String = source
        .lines()
        .enumerate()
        .filter(|(i, _)| *i >= context_start && *i < context_end)
        .map(|(i, l)| format!("{:4}: {l}\n", i + 1))
        .collect();

    format!(
        "Fix this clippy warning in {}:\n\
         Lint: {}\n\
         Message: {}\n\n\
         Relevant source (lines {}-{}):\n\
         ```rust\n{snippet}```\n\n\
         Output a unified diff that fixes the warning.",
        finding.file.display(),
        finding.lint,
        finding.message,
        context_start + 1,
        context_end,
    )
}

fn extract_diff(text: &str) -> String {
    // Accept either a raw diff or one wrapped in a ```diff ... ``` block
    if let Some(start) = text.find("--- ") {
        return text[start..].to_string();
    }
    if let Some(start) = text.find("```diff\n") {
        let inner = &text[start + 8..];
        if let Some(end) = inner.find("\n```") {
            return inner[..end].to_string();
        }
    }
    String::new()
}

// ─── LLMEditStrategy ────────────────────────────────────────────────

/// Asks the LLM to rewrite a compiler-indicated byte range.
///
/// Because the compiler tells us exactly where the problem is, the LLM only
/// needs to fix a small, well-defined slice of code — no guessing required.
pub struct LLMEditStrategy {
    pub llm: Box<dyn LLMClient>,
    pub model: String,
}

#[async_trait]
impl EditStrategy for LLMEditStrategy {
    async fn apply(&self, finding: &ClippyFinding, workspace: &Path) -> EditResult {
        let (byte_start, byte_end) = match (finding.byte_start, finding.byte_end) {
            (Some(s), Some(e)) => (s as usize, e as usize),
            _ => return EditResult::err(&finding.file, "no byte range in finding"),
        };

        let file_path = workspace.join(&finding.file);
        let source = match tokio::fs::read_to_string(&file_path).await {
            Ok(s) => s,
            Err(e) => return EditResult::err(&finding.file, format!("read error: {e}")),
        };

        if byte_end > source.len() || byte_start > byte_end {
            return EditResult::err(&finding.file, "byte range out of bounds");
        }

        let target_slice = &source[byte_start..byte_end];
        let prompt = build_edit_prompt(finding, target_slice);
        let messages = vec![
            ChatMessage::system(EDIT_SYSTEM_PROMPT),
            ChatMessage::user(prompt),
        ];

        let response = match self.llm.chat(messages, &self.model, 0.1, 1024).await {
            Ok(r) => r.content,
            Err(e) => return EditResult::err(&finding.file, format!("LLM error: {e}")),
        };

        let replacement = extract_replacement(&response);
        if replacement.is_empty() {
            return EditResult::err(&finding.file, "LLM returned empty replacement");
        }

        let mut new_source = source.clone();
        new_source.replace_range(byte_start..byte_end, &replacement);

        if let Err(e) = tokio::fs::write(&file_path, new_source).await {
            return EditResult::err(&finding.file, format!("write error: {e}"));
        }

        EditResult::ok(
            &finding.file,
            format!("replaced bytes {byte_start}..{byte_end} with LLM suggestion"),
        )
    }
}

const EDIT_SYSTEM_PROMPT: &str = "\
You are a Rust expert fixing a clippy warning. \
Output ONLY the replacement code snippet — nothing else. \
No markdown, no explanation, no surrounding context.";

fn build_edit_prompt(finding: &ClippyFinding, target: &str) -> String {
    format!(
        "Fix this clippy warning:\n\
         Lint: {}\n\
         Message: {}\n\n\
         The flagged code is:\n\
         ```rust\n{target}\n```\n\n\
         Output only the replacement (the fixed version of the code above).",
        finding.lint, finding.message,
    )
}

fn extract_replacement(text: &str) -> String {
    let text = text.trim();
    // Strip markdown code block if present
    if let Some(inner) = text
        .strip_prefix("```rust\n")
        .or_else(|| text.strip_prefix("```\n"))
    {
        if let Some(body) = inner
            .strip_suffix("\n```")
            .or_else(|| inner.strip_suffix("```"))
        {
            return body.to_string();
        }
    }
    text.to_string()
}

// ─── Pipeline-internal dispatch ──────────────────────────────────────

use super::risk::RiskLevel;

/// Dispatch a finding to the right edit strategy without requiring an owned LLM.
///
/// Called by `AutonomousImprovementPipeline` which only holds `&dyn LLMClient`.
/// Falls back to `ClippyFixStrategy` for `Patch` when the suggestion is
/// `MachineApplicable` (safe to let the compiler apply it directly).
pub async fn dispatch(
    finding: &ClippyFinding,
    risk: RiskLevel,
    workspace: &Path,
    llm: &dyn LLMClient,
    model: &str,
) -> EditResult {
    match risk {
        RiskLevel::AutoFix => ClippyFixStrategy.apply(finding, workspace).await,
        RiskLevel::Patch => apply_diff_patch(finding, workspace, llm, model).await,
        RiskLevel::LLMEdit => apply_llm_edit(finding, workspace, llm, model).await,
        RiskLevel::HumanReview => {
            EditResult::err(&finding.file, "human review required — skipping")
        }
    }
}

async fn apply_diff_patch(
    finding: &ClippyFinding,
    workspace: &Path,
    llm: &dyn LLMClient,
    model: &str,
) -> EditResult {
    let file_path = workspace.join(&finding.file);
    let source = match tokio::fs::read_to_string(&file_path).await {
        Ok(s) => s,
        Err(e) => return EditResult::err(&finding.file, format!("read error: {e}")),
    };

    let prompt = build_diff_prompt(finding, &source);
    let messages = vec![
        ChatMessage::system(DIFF_SYSTEM_PROMPT),
        ChatMessage::user(prompt),
    ];
    let response = match llm.chat(messages, model, 0.1, 2048).await {
        Ok(r) => r.content,
        Err(e) => return EditResult::err(&finding.file, format!("LLM error: {e}")),
    };

    let patch = extract_diff(&response);
    if patch.is_empty() {
        return EditResult::err(&finding.file, "LLM returned no unified diff");
    }

    let parsed = match diffy::Patch::from_str(&patch) {
        Ok(p) => p,
        Err(e) => return EditResult::err(&finding.file, format!("patch parse failed: {e}")),
    };
    match diffy::apply(&source, &parsed) {
        Ok(patched) => {
            if let Err(e) = tokio::fs::write(&file_path, patched).await {
                return EditResult::err(&finding.file, format!("write error: {e}"));
            }
            EditResult::ok(&finding.file, "diff patch applied")
        }
        Err(e) => EditResult::err(&finding.file, format!("patch apply failed: {e}")),
    }
}

async fn apply_llm_edit(
    finding: &ClippyFinding,
    workspace: &Path,
    llm: &dyn LLMClient,
    model: &str,
) -> EditResult {
    let (byte_start, byte_end) = match (finding.byte_start, finding.byte_end) {
        (Some(s), Some(e)) => (s as usize, e as usize),
        _ => return EditResult::err(&finding.file, "no byte range in finding"),
    };

    let file_path = workspace.join(&finding.file);
    let source = match tokio::fs::read_to_string(&file_path).await {
        Ok(s) => s,
        Err(e) => return EditResult::err(&finding.file, format!("read error: {e}")),
    };

    if byte_end > source.len() || byte_start > byte_end {
        return EditResult::err(&finding.file, "byte range out of bounds");
    }

    let target_slice = &source[byte_start..byte_end];
    let prompt = build_edit_prompt(finding, target_slice);
    let messages = vec![
        ChatMessage::system(EDIT_SYSTEM_PROMPT),
        ChatMessage::user(prompt),
    ];
    let response = match llm.chat(messages, model, 0.1, 1024).await {
        Ok(r) => r.content,
        Err(e) => return EditResult::err(&finding.file, format!("LLM error: {e}")),
    };

    let replacement = extract_replacement(&response);
    if replacement.is_empty() {
        return EditResult::err(&finding.file, "LLM returned empty replacement");
    }

    let mut new_source = source.clone();
    new_source.replace_range(byte_start..byte_end, &replacement);
    if let Err(e) = tokio::fs::write(&file_path, new_source).await {
        return EditResult::err(&finding.file, format!("write error: {e}"));
    }
    EditResult::ok(
        &finding.file,
        format!("replaced bytes {byte_start}..{byte_end} with LLM suggestion"),
    )
}
