//! ImprovementPlanner — converts research + review data into
//! a prioritized list of concrete, file-level improvement candidates.

use lab_core::llm::{ChatMessage, LLMClient};
use serde::{Deserialize, Serialize};

// ─── Candidate ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementCandidate {
    /// Path to the file (relative to workspace)
    pub file: String,
    /// Specific, single-file improvement task
    pub task: String,
    /// "high" | "medium" | "low"
    pub priority: String,
    /// Why this matters
    pub rationale: String,
}

// ─── Planner ─────────────────────────────────────────────────────

/// Ask the LLM to produce improvement candidates from research + review data.
///
/// Returns up to `max_candidates` candidates, parsed from `<candidates>...</candidates>`.
pub async fn plan_improvements(
    researcher_data: Option<&serde_json::Value>,
    review_data: Option<&serde_json::Value>,
    web_context: Option<&str>,
    llm: &dyn LLMClient,
    model: &str,
    max_candidates: usize,
) -> Vec<ImprovementCandidate> {
    let context = build_context(researcher_data, review_data, web_context);

    let system = "\
You are a senior Rust engineer auditing a codebase called AI Research Lab.\n\
Based on the provided analysis, generate a prioritized list of specific, atomic, \
single-file improvements that can be implemented by editing one file at a time.\n\
\n\
Output ONLY a JSON array wrapped in <candidates></candidates> tags.\n\
Each item: {\"file\":\"path\",\"task\":\"specific task\",\"priority\":\"high|medium|low\",\
\"rationale\":\"why\"}\n\
\n\
Priority guide:\n\
  high   — safety issues: .unwrap()/.expect() without context, panic!, unsafe blocks\n\
  medium — quality: missing docs, unclear naming, redundant clones, missing error context\n\
  low    — style: long lines, TODO comments, minor refactors\n\
\n\
Rules:\n\
- Each task must be implementable by editing ONE file only\n\
- Be specific — cite the exact function or pattern to change\n\
- Do NOT suggest adding features or architectural refactors\n\
- Do NOT suggest changes to test files\n\
- Output ONLY the <candidates>...</candidates> block";

    let prompt = format!("Generate up to {max_candidates} improvement candidates.\n\n{context}");

    match llm
        .chat(
            vec![ChatMessage::system(system), ChatMessage::user(prompt)],
            model,
            0.2,
            2048,
        )
        .await
    {
        Ok(r) => parse_candidates(&r.content),
        Err(e) => {
            tracing::warn!("ImprovementPlanner LLM call failed: {e}");
            Vec::new()
        }
    }
}

// ─── Context Builder ─────────────────────────────────────────────

fn build_context(
    researcher_data: Option<&serde_json::Value>,
    review_data: Option<&serde_json::Value>,
    web_context: Option<&str>,
) -> String {
    let mut ctx = String::with_capacity(4096);

    // Researcher summary
    if let Some(r) = researcher_data {
        let files = r
            .get("files_analysed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let classes = r.get("total_classes").and_then(|v| v.as_u64()).unwrap_or(0);
        let fns = r
            .get("total_functions")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        ctx.push_str(&format!(
            "## Codebase Overview\nFiles: {files} | Classes/Structs: {classes} | Functions: {fns}\n\n"
        ));

        // Top files by function density
        if let Some(arr) = r.get("files").and_then(|v| v.as_array()) {
            let mut files_vec: Vec<_> = arr.iter().collect();
            files_vec.sort_by(|a, b| {
                let fa = a
                    .get("async_functions")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    + a.get("sync_functions")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                let fb = b
                    .get("async_functions")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    + b.get("sync_functions")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                fb.cmp(&fa)
            });
            ctx.push_str("### Most complex files:\n");
            for f in files_vec.iter().take(12) {
                let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                let lines = f.get("total_lines").and_then(|v| v.as_u64()).unwrap_or(0);
                let afns = f
                    .get("async_functions")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let sfns = f
                    .get("sync_functions")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                ctx.push_str(&format!(
                    "- {path}  ({lines}L, {afns} async + {sfns} sync fns)\n"
                ));
            }
            ctx.push('\n');
        }
    }

    // Reviewer summary
    if let Some(rv) = review_data {
        ctx.push_str("## Code Quality Issues\n");
        if let Some(s) = rv.get("summary") {
            let total = s.get("total_issues").and_then(|v| v.as_u64()).unwrap_or(0);
            let reviewed = s
                .get("files_reviewed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            ctx.push_str(&format!(
                "Total: {total} issues across {reviewed} files\n\n"
            ));

            if let Some(by_rule) = s.get("by_rule").and_then(|v| v.as_object()) {
                ctx.push_str("By rule:\n");
                let mut rules: Vec<_> = by_rule.iter().collect();
                rules.sort_by(|a, b| b.1.as_u64().unwrap_or(0).cmp(&a.1.as_u64().unwrap_or(0)));
                for (rule, count) in rules.iter().take(8) {
                    ctx.push_str(&format!("  {rule}: {count}\n"));
                }
                ctx.push('\n');
            }
        }

        // Files with most issues
        if let Some(reviews) = rv.get("reviews").and_then(|v| v.as_array()) {
            let mut sorted: Vec<_> = reviews.iter().collect();
            sorted.sort_by(|a, b| {
                let ac = a.get("issue_count").and_then(|v| v.as_u64()).unwrap_or(0);
                let bc = b.get("issue_count").and_then(|v| v.as_u64()).unwrap_or(0);
                bc.cmp(&ac)
            });
            ctx.push_str("### Files with most issues:\n");
            for r in sorted.iter().take(10) {
                let path = r.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                let n = r.get("issue_count").and_then(|v| v.as_u64()).unwrap_or(0);
                if n > 0 {
                    // List specific issues
                    let issue_list: Vec<String> = r
                        .get("issues")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .take(3)
                                .filter_map(|i| {
                                    let rule = i.get("rule").and_then(|v| v.as_str())?;
                                    let line = i.get("line").and_then(|v| v.as_u64())?;
                                    Some(format!("{rule}@L{line}"))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    ctx.push_str(&format!(
                        "- {path}  ({n} issues: {})\n",
                        issue_list.join(", ")
                    ));
                }
            }
            ctx.push('\n');
        }
    }

    // Web context
    if let Some(web) = web_context {
        let snippet: String = web.chars().take(1200).collect();
        ctx.push_str("## External Research\n");
        ctx.push_str(&snippet);
        ctx.push('\n');
    }

    ctx
}

// ─── Parser ──────────────────────────────────────────────────────

fn parse_candidates(text: &str) -> Vec<ImprovementCandidate> {
    let open = "<candidates>";
    let close = "</candidates>";
    let start = match text.find(open) {
        Some(i) => i + open.len(),
        None => return Vec::new(),
    };
    let end = match text[start..].find(close) {
        Some(i) => start + i,
        None => return Vec::new(),
    };
    serde_json::from_str::<Vec<ImprovementCandidate>>(text[start..end].trim()).unwrap_or_default()
}
