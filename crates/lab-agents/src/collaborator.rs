//! MultiAgentCollaborator — orchestrates multiple agents in a pipeline.
//! Mirrors agents/collaborator.py (252 lines)

use crate::coder::CoderAgent;
use crate::researcher::ResearcherAgent;
use crate::reviewer::ReviewerAgent;
use crate::summarizer::SummarizerAgent;
use lab_core::types::AgentResult;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use tracing::{info, warn};

// ─── Status constants ───────────────────────────────────────

pub mod status {
    pub const IDLE: &str = "idle";
    pub const RUNNING: &str = "running";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
}

// ─── PhaseResult ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PhaseResult {
    pub name: String,
    pub success: bool,
    pub data: serde_json::Value,
    pub duration_secs: f64,
    pub agent_id: String,
    pub error: String,
}

// ─── WorkflowResult ─────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WorkflowResult {
    pub status: String,
    pub phases: Vec<PhaseResult>,
    pub total_duration_secs: f64,
    pub error: String,
}

impl WorkflowResult {
    pub fn success(&self) -> bool {
        self.status == status::COMPLETED && self.phases.iter().all(|p| p.success)
    }
}

// ─── MultiAgentCollaborator ─────────────────────────────────

/// Runs a pipeline of agents that collaborate on a single task.
///
/// Typical flow: Research → Review → Code → Summarize
/// Each phase writes to memory so the next phase can read it.
pub struct MultiAgentCollaborator {
    pub session_id: String,
    pub status: String,
    run_review: bool,
    run_code: bool,
    run_summary: bool,
}

impl MultiAgentCollaborator {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            status: status::IDLE.to_string(),
            run_review: true,
            run_code: true,
            run_summary: true,
        }
    }

    pub fn enable_review(mut self, flag: bool) -> Self {
        self.run_review = flag;
        self
    }

    pub fn enable_code_generation(mut self, flag: bool) -> Self {
        self.run_code = flag;
        self
    }

    pub fn enable_summary(mut self, flag: bool) -> Self {
        self.run_summary = flag;
        self
    }

    /// Execute the full multi-agent pipeline.
    pub async fn run(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        pattern: Option<&str>,
        path: Option<&str>,
    ) -> WorkflowResult {
        let start = Instant::now();
        self.status = status::RUNNING.to_string();

        let mut researcher = ResearcherAgent::new(
            format!("researcher-{}", &uuid::Uuid::new_v4().to_string()[..6]),
            self.session_id.clone(),
            lab_core::config::AgentProfile::default(),
            std::path::PathBuf::from("."),
        );

        let mut phases = Vec::new();

        // Phase 1: Research — always runs
        let phase_start = Instant::now();
        let research_result = researcher
            .execute(registry, memory, "analyze codebase", pattern, path, None)
            .await;

        phases.push(PhaseResult {
            name: "research".into(),
            success: research_result.success,
            data: research_result.data.clone(),
            duration_secs: phase_start.elapsed().as_secs_f64(),
            agent_id: researcher.id().to_string(),
            error: research_result.error.unwrap_or_default(),
        });

        if !research_result.success {
            self.status = status::FAILED.to_string();
            return self.finish(phases, start.elapsed().as_secs_f64(), "Research phase failed".into());
        }

        info!("Collaborator: Research phase completed ({:.1}s)", phases[0].duration_secs);

        // Phase 2: Review — optional
        if self.run_review {
            let phase_start = Instant::now();
            let mut reviewer = ReviewerAgent::new(
                format!("reviewer-{}", &uuid::Uuid::new_v4().to_string()[..6]),
                self.session_id.clone(),
                lab_core::config::AgentProfile::default(),
                std::path::PathBuf::from("."),
            );

            let review_result = reviewer
                .execute(registry, memory, "review code quality", pattern, path, None)
                .await;

            phases.push(PhaseResult {
                name: "review".into(),
                success: review_result.success,
                data: review_result.data.clone(),
                duration_secs: phase_start.elapsed().as_secs_f64(),
                agent_id: reviewer.id().to_string(),
                error: review_result.error.unwrap_or_default(),
            });

            if !review_result.success {
                warn!("Collaborator: Review phase failed — continuing");
            }
        }

        // Phase 3: Code — optional
        if self.run_code {
            let phase_start = Instant::now();
            let mut coder = CoderAgent::new(
                format!("coder-{}", &uuid::Uuid::new_v4().to_string()[..6]),
                self.session_id.clone(),
                lab_core::config::AgentProfile::default(),
                std::path::PathBuf::from("."),
            );

            let code_result = coder
                .execute(registry, memory, "generate boilerplate", None, None, Some("module"))
                .await;

            phases.push(PhaseResult {
                name: "code".into(),
                success: code_result.success,
                data: code_result.data.clone(),
                duration_secs: phase_start.elapsed().as_secs_f64(),
                agent_id: coder.id().to_string(),
                error: code_result.error.unwrap_or_default(),
            });
        }

        // Phase 4: Summary — optional
        if self.run_summary {
            let phase_start = Instant::now();
            let mut summarizer = SummarizerAgent::new(
                format!("summarizer-{}", &uuid::Uuid::new_v4().to_string()[..6]),
                self.session_id.clone(),
                lab_core::config::AgentProfile::default(),
                std::path::PathBuf::from("."),
            );

            let summary_result = summarizer
                .execute(registry, memory, "generate summary", Some("lab-outputs/collaboration-summary.md"))
                .await;

            phases.push(PhaseResult {
                name: "summary".into(),
                success: summary_result.success,
                data: summary_result.data.clone(),
                duration_secs: phase_start.elapsed().as_secs_f64(),
                agent_id: summarizer.id().to_string(),
                error: summary_result.error.unwrap_or_default(),
            });
        }

        self.status = status::COMPLETED.to_string();
        let total = start.elapsed().as_secs_f64();
        info!(
            "Collaborator: pipeline completed in {:.1}s ({} phases)",
            total,
            phases.len()
        );

        WorkflowResult {
            status: self.status.clone(),
            phases,
            total_duration_secs: total,
            error: String::new(),
        }
    }

    fn finish(
        &self,
        phases: Vec<PhaseResult>,
        total_secs: f64,
        error: String,
    ) -> WorkflowResult {
        WorkflowResult {
            status: self.status.clone(),
            phases,
            total_duration_secs: total_secs,
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collaborator_creation() {
        let c = MultiAgentCollaborator::new("test-session");
        assert_eq!(c.status, status::IDLE);
        assert!(c.run_review);
        assert!(c.run_code);
        assert!(c.run_summary);
    }

    #[test]
    fn collaborator_flags() {
        let c = MultiAgentCollaborator::new("s1")
            .enable_review(false)
            .enable_code_generation(false);
        assert!(!c.run_review);
        assert!(!c.run_code);
        assert!(c.run_summary);
    }

    #[test]
    fn workflow_result_success() {
        let wr = WorkflowResult {
            status: status::COMPLETED.to_string(),
            phases: vec![
                PhaseResult {
                    name: "research".into(),
                    success: true,
                    data: serde_json::json!({}),
                    duration_secs: 1.0,
                    agent_id: "r1".into(),
                    error: String::new(),
                },
            ],
            total_duration_secs: 1.0,
            error: String::new(),
        };
        assert!(wr.success());
    }
}
