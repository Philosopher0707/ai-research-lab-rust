//! AutonomousImprovementPipeline — full self-improvement loop.
//!
//! Stages:
//!   1. Research  — ResearcherAgent + ReviewerAgent analyze the workspace
//!   2. Plan      — ImprovementPlanner turns findings into concrete candidates
//!   3. Implement — SelfEditAgent applies each candidate, cargo check verifies
//!   4. (Optional) cargo test per change
//!   5. Git commit each successful change on a dedicated branch

use crate::improvement_planner::{plan_improvements, ImprovementCandidate};
use crate::researcher::ResearcherAgent;
use crate::reviewer::ReviewerAgent;
use crate::self_edit::SelfEditAgent;
use lab_core::config::AgentProfile;
use lab_core::llm::LLMClient;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{info, warn};

// ─── Config ──────────────────────────────────────────────────────

pub struct ImprovementConfig {
    /// Glob pattern for files to analyse
    pub pattern: String,
    /// How many candidates to ask the planner for
    pub max_candidates: usize,
    /// How many to actually attempt per run
    pub max_per_run: usize,
    /// If true: plan and show candidates but do not apply
    pub dry_run: bool,
    /// Run `cargo test` after each successful edit (slower but thorough)
    pub run_tests: bool,
    /// Create a dedicated git branch for this improvement session
    pub create_branch: bool,
    /// Fetch web context before planning
    pub web_research: bool,
}

impl Default for ImprovementConfig {
    fn default() -> Self {
        Self {
            pattern: "**/*.rs".into(),
            max_candidates: 10,
            max_per_run: 5,
            dry_run: false,
            run_tests: false,
            create_branch: true,
            web_research: false,
        }
    }
}

// ─── Result Types ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ImprovementAttempt {
    pub file: String,
    pub task: String,
    pub priority: String,
    pub edits_applied: usize,
    pub success: bool,
    pub reverted: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImprovementReport {
    pub session_id: String,
    pub total_planned: usize,
    pub applied: usize,
    pub failed: usize,
    pub skipped: usize,
    pub git_branch: Option<String>,
    pub duration_secs: f64,
    pub attempts: Vec<ImprovementAttempt>,
    /// Planned candidates that were not attempted (past max_per_run)
    pub remaining: Vec<ImprovementCandidate>,
}

// ─── Pipeline ────────────────────────────────────────────────────

pub struct AutonomousImprovementPipeline {
    pub session_id: String,
    workspace: PathBuf,
    pub config: ImprovementConfig,
}

impl AutonomousImprovementPipeline {
    pub fn new(session_id: impl Into<String>, workspace: PathBuf, config: ImprovementConfig) -> Self {
        Self {
            session_id: session_id.into(),
            workspace,
            config,
        }
    }

    /// Run the full autonomous improvement loop.
    pub async fn run(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        llm: &dyn LLMClient,
        model: &str,
    ) -> ImprovementReport {
        let start = Instant::now();

        // ── 1. Research ──────────────────────────────────────
        info!("[improve] Phase 1: research");
        self.run_researcher(registry, memory, llm, model).await;

        // ── 2. Review ────────────────────────────────────────
        info!("[improve] Phase 2: review");
        self.run_reviewer(registry, memory, llm, model).await;

        let researcher_data = memory.get(&self.session_id, "researcher_map");
        let review_data = memory.get(&self.session_id, "review_results");

        // ── 3. Optional web research ─────────────────────────
        let web_ctx = if self.config.web_research {
            info!("[improve] Phase 3: web research");
            self.fetch_web_context(registry, &review_data).await
        } else {
            None
        };

        // ── 4. Plan ──────────────────────────────────────────
        info!("[improve] Phase 4: planning");
        let all_candidates = plan_improvements(
            researcher_data.as_ref(),
            review_data.as_ref(),
            web_ctx.as_deref(),
            llm,
            model,
            self.config.max_candidates,
        )
        .await;

        let total_planned = all_candidates.len();

        if all_candidates.is_empty() {
            warn!("[improve] Planner returned 0 candidates — LLM may not have responded correctly");
            return ImprovementReport {
                session_id: self.session_id.clone(),
                total_planned: 0,
                applied: 0,
                failed: 0,
                skipped: 0,
                git_branch: None,
                duration_secs: start.elapsed().as_secs_f64(),
                attempts: Vec::new(),
                remaining: Vec::new(),
            };
        }

        // Split into active batch + remaining
        let (active, remaining) = if all_candidates.len() > self.config.max_per_run {
            let rest = all_candidates[self.config.max_per_run..].to_vec();
            (all_candidates.into_iter().take(self.config.max_per_run).collect::<Vec<_>>(), rest)
        } else {
            (all_candidates, Vec::new())
        };

        // ── Dry run early exit ────────────────────────────────
        if self.config.dry_run {
            let attempts = active
                .iter()
                .map(|c| ImprovementAttempt {
                    file: c.file.clone(),
                    task: c.task.clone(),
                    priority: c.priority.clone(),
                    edits_applied: 0,
                    success: false,
                    reverted: false,
                    error: Some("dry_run".into()),
                })
                .collect();
            return ImprovementReport {
                session_id: self.session_id.clone(),
                total_planned,
                applied: 0,
                failed: 0,
                skipped: active.len(),
                git_branch: None,
                duration_secs: start.elapsed().as_secs_f64(),
                attempts,
                remaining,
            };
        }

        // ── 5. Create git branch ──────────────────────────────
        let git_branch = if self.config.create_branch {
            info!("[improve] Creating git branch");
            Some(self.create_git_branch(registry).await)
        } else {
            None
        };

        // ── 6. Apply improvements ─────────────────────────────
        info!("[improve] Phase 5: applying {} candidate(s)", active.len());
        let mut attempts: Vec<ImprovementAttempt> = Vec::new();
        let mut applied = 0usize;
        let mut failed = 0usize;

        for (idx, candidate) in active.iter().enumerate() {
            info!(
                "[improve] [{}/{}] {} — {}",
                idx + 1,
                active.len(),
                candidate.file,
                &candidate.task[..candidate.task.len().min(60)]
            );

            let agent_id = format!(
                "self-edit-{}",
                &uuid::Uuid::new_v4().to_string()[..6]
            );
            let mut agent = SelfEditAgent::new(
                agent_id,
                self.session_id.clone(),
                AgentProfile::default(),
                self.workspace.clone(),
            );

            let result = agent
                .execute(
                    registry,
                    memory,
                    &candidate.file,
                    &candidate.task,
                    false,
                    llm,
                    model,
                )
                .await;

            if result.success {
                let edits = result
                    .data
                    .get("edits_applied")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;

                if edits == 0 {
                    // LLM proposed no changes — skip without counting as failure
                    attempts.push(ImprovementAttempt {
                        file: candidate.file.clone(),
                        task: candidate.task.clone(),
                        priority: candidate.priority.clone(),
                        edits_applied: 0,
                        success: false,
                        reverted: false,
                        error: Some("no edits proposed".into()),
                    });
                    continue;
                }

                // Optional: run cargo test
                let mut ok = true;
                if self.config.run_tests {
                    ok = self.run_tests(registry).await;
                    if !ok {
                        // Revert
                        self.revert_file(registry, &candidate.file).await;
                        failed += 1;
                        attempts.push(ImprovementAttempt {
                            file: candidate.file.clone(),
                            task: candidate.task.clone(),
                            priority: candidate.priority.clone(),
                            edits_applied: edits,
                            success: false,
                            reverted: true,
                            error: Some("cargo test failed — reverted".into()),
                        });
                        continue;
                    }
                }

                // Commit the change
                if ok && git_branch.is_some() {
                    self.commit_change(registry, &candidate.file, &candidate.task).await;
                }

                applied += 1;
                attempts.push(ImprovementAttempt {
                    file: candidate.file.clone(),
                    task: candidate.task.clone(),
                    priority: candidate.priority.clone(),
                    edits_applied: edits,
                    success: true,
                    reverted: false,
                    error: None,
                });
            } else {
                failed += 1;
                attempts.push(ImprovementAttempt {
                    file: candidate.file.clone(),
                    task: candidate.task.clone(),
                    priority: candidate.priority.clone(),
                    edits_applied: 0,
                    success: false,
                    reverted: true,
                    error: result.error,
                });
            }
        }

        ImprovementReport {
            session_id: self.session_id.clone(),
            total_planned,
            applied,
            failed,
            skipped: remaining.len(),
            git_branch,
            duration_secs: start.elapsed().as_secs_f64(),
            attempts,
            remaining,
        }
    }

    // ─── Phase helpers ───────────────────────────────────────────

    async fn run_researcher(
        &self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        llm: &dyn LLMClient,
        model: &str,
    ) {
        let mut agent = ResearcherAgent::new(
            format!("researcher-{}", &uuid::Uuid::new_v4().to_string()[..6]),
            self.session_id.clone(),
            AgentProfile::default(),
            self.workspace.clone(),
        );
        let result = agent
            .execute(
                registry,
                memory,
                "analyze workspace for autonomous improvement",
                Some(&self.config.pattern),
                None,
                Some(100),
                Some(llm),
                Some(model),
            )
            .await;
        if !result.success {
            warn!("[improve] ResearcherAgent failed: {:?}", result.error);
        }
    }

    async fn run_reviewer(
        &self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        llm: &dyn LLMClient,
        model: &str,
    ) {
        let mut agent = ReviewerAgent::new(
            format!("reviewer-{}", &uuid::Uuid::new_v4().to_string()[..6]),
            self.session_id.clone(),
            AgentProfile::default(),
            self.workspace.clone(),
        );
        let result = agent
            .execute(
                registry,
                memory,
                "review code quality for autonomous improvement",
                Some(&self.config.pattern),
                None,
                Some(50),
                Some(llm),
                Some(model),
            )
            .await;
        if !result.success {
            warn!("[improve] ReviewerAgent failed: {:?}", result.error);
        }
    }

    async fn fetch_web_context(
        &self,
        registry: &mut ToolRegistry,
        review_data: &Option<serde_json::Value>,
    ) -> Option<String> {
        // Build a query from the top issue rule
        let top_rule = review_data
            .as_ref()
            .and_then(|rv| rv.get("summary"))
            .and_then(|s| s.get("by_rule"))
            .and_then(|v| v.as_object())
            .and_then(|obj| {
                obj.iter()
                    .max_by_key(|(_, v)| v.as_u64().unwrap_or(0))
                    .map(|(k, _)| k.clone())
            })
            .unwrap_or_else(|| "unsafe_unwrap".to_string());

        let query = format!("Rust best practice {top_rule} fix example");
        let result = registry
            .execute(
                "web_search",
                &HashMap::from([
                    ("query".into(), serde_json::json!(query)),
                    ("max_results".into(), serde_json::json!(3)),
                ]),
            )
            .await;

        if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            let snippets: Vec<String> = result
                .get("data")
                .and_then(|d| d.get("results"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .take(3)
                        .filter_map(|item| {
                            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                            let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
                            if snippet.is_empty() {
                                None
                            } else {
                                Some(format!("• {title}: {snippet}"))
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(snippets.join("\n"))
        } else {
            None
        }
    }

    // ─── Git helpers ─────────────────────────────────────────────

    async fn create_git_branch(&self, registry: &mut ToolRegistry) -> String {
        let branch = format!("lab/improve-{}", &self.session_id[..8]);
        let ws = shell_escape(self.workspace.to_string_lossy().as_ref());
        // Create branch; if it already exists just switch to it
        let cmd = format!(
            "git -C {ws} checkout -b {branch} 2>/dev/null || git -C {ws} checkout {branch} 2>&1"
        );
        let r = registry
            .execute(
                "bash",
                &HashMap::from([("command".into(), serde_json::json!(cmd))]),
            )
            .await;
        if r.get("data")
            .and_then(|d| d.get("returncode"))
            .and_then(|v| v.as_i64())
            .unwrap_or(1)
            == 0
        {
            info!("[improve] On branch {branch}");
        } else {
            warn!("[improve] Could not create branch {branch}");
        }
        branch
    }

    async fn commit_change(&self, registry: &mut ToolRegistry, file: &str, task: &str) {
        let ws = shell_escape(self.workspace.to_string_lossy().as_ref());
        let file_name = std::path::Path::new(file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(file);
        let msg = format!(
            "lab: {file_name}: {}",
            task.chars().take(72).collect::<String>()
        );
        let cmd = format!(
            "git -C {ws} add -A && git -C {ws} commit -m {}",
            shell_escape(&msg)
        );
        registry
            .execute(
                "bash",
                &HashMap::from([("command".into(), serde_json::json!(cmd))]),
            )
            .await;
    }

    async fn revert_file(&self, registry: &mut ToolRegistry, file: &str) {
        let ws = shell_escape(self.workspace.to_string_lossy().as_ref());
        let fp = shell_escape(file);
        let cmd = format!("git -C {ws} checkout -- {fp}");
        registry
            .execute(
                "bash",
                &HashMap::from([("command".into(), serde_json::json!(cmd))]),
            )
            .await;
    }

    async fn run_tests(&self, registry: &mut ToolRegistry) -> bool {
        let ws = shell_escape(self.workspace.to_string_lossy().as_ref());
        let cmd = format!(
            "cd {ws} && cargo test --quiet 2>&1 | tail -20"
        );
        let r = registry
            .execute(
                "bash",
                &HashMap::from([("command".into(), serde_json::json!(cmd))]),
            )
            .await;
        r.get("data")
            .and_then(|d| d.get("returncode"))
            .and_then(|v| v.as_i64())
            .map(|code| code == 0)
            .unwrap_or(false)
    }
}

// ─── Shell Escaping ───────────────────────────────────────────────

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
