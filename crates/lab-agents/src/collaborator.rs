//! MultiAgentCollaborator — generalized orchestration for agent teams.

use crate::runtime::{
    execute_agent_with_context, AgentExecutionContext, AgentExecutionRequest, AgentFactoryRegistry,
};
use lab_core::config::AgentProfile;
use lab_core::llm::LLMClient;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use serde::Serialize;
use std::time::Instant;
use tracing::{info, warn};

// ─── Status constants ───────────────────────────────────────

pub mod status {
    pub const IDLE: &str = "idle";
    pub const RUNNING: &str = "running";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
}

// ─── Phase / Workflow Result Types ──────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CollaborationPhase {
    pub name: String,
    pub agent_type: String,
    pub task: String,
    pub continue_on_failure: bool,
    pub pattern: Option<String>,
    pub path: Option<String>,
    pub output_path: Option<String>,
    pub content: Option<String>,
    pub template: Option<String>,
    pub profile: AgentProfile,
}

impl CollaborationPhase {
    pub fn new(
        name: impl Into<String>,
        agent_type: impl Into<String>,
        task: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            agent_type: agent_type.into(),
            task: task.into(),
            continue_on_failure: true,
            pattern: None,
            path: None,
            output_path: None,
            content: None,
            template: None,
            profile: AgentProfile::default(),
        }
    }

    fn into_request(
        &self,
        session_id: &str,
        inherited_pattern: Option<&str>,
        inherited_path: Option<&str>,
    ) -> AgentExecutionRequest {
        AgentExecutionRequest {
            agent_type: self.agent_type.clone(),
            agent_id: format!(
                "{}-{}",
                self.agent_type,
                &uuid::Uuid::new_v4().to_string()[..6]
            ),
            session_id: session_id.to_string(),
            task: self.task.clone(),
            pattern: self
                .pattern
                .clone()
                .or_else(|| inherited_pattern.map(str::to_string)),
            path: self.path.clone().or_else(|| inherited_path.map(str::to_string)),
            output_path: self.output_path.clone(),
            content: self.content.clone(),
            template: self.template.clone(),
            profile: self.profile.clone(),
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseResult {
    pub name: String,
    pub success: bool,
    pub data: serde_json::Value,
    pub duration_secs: f64,
    pub agent_id: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct WorkflowResult {
    pub status: String,
    pub phases: Vec<PhaseResult>,
    pub total_duration_secs: f64,
    pub error: String,
}

impl WorkflowResult {
    pub fn success(&self) -> bool {
        self.status == status::COMPLETED && self.phases.iter().all(|phase| phase.success)
    }
}

// ─── MultiAgentCollaborator ─────────────────────────────────

pub struct MultiAgentCollaborator {
    pub session_id: String,
    pub status: String,
    phases: Vec<CollaborationPhase>,
}

impl MultiAgentCollaborator {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            status: status::IDLE.to_string(),
            phases: Self::default_phases(),
        }
    }

    fn default_phases() -> Vec<CollaborationPhase> {
        let mut research = CollaborationPhase::new("research", "researcher", "analyze codebase");
        research.continue_on_failure = false;

        let review = CollaborationPhase::new("review", "reviewer", "review code quality");

        let mut code = CollaborationPhase::new("code", "coder", "generate boilerplate");
        code.template = Some("module".to_string());

        let mut summary =
            CollaborationPhase::new("summary", "summarizer", "generate summary");
        summary.output_path = Some("lab-outputs/collaboration-summary.md".to_string());

        vec![research, review, code, summary]
    }

    pub fn phases(&self) -> &[CollaborationPhase] {
        &self.phases
    }

    pub fn with_phase(mut self, phase: CollaborationPhase) -> Self {
        self.phases.push(phase);
        self
    }

    pub fn replace_phases(mut self, phases: Vec<CollaborationPhase>) -> Self {
        self.phases = phases;
        self
    }

    pub fn enable_review(mut self, flag: bool) -> Self {
        self.set_phase_enabled("review", flag);
        self
    }

    pub fn enable_code_generation(mut self, flag: bool) -> Self {
        self.set_phase_enabled("code", flag);
        self
    }

    pub fn enable_summary(mut self, flag: bool) -> Self {
        self.set_phase_enabled("summary", flag);
        self
    }

    fn set_phase_enabled(&mut self, phase_name: &str, enabled: bool) {
        if enabled {
            if !self.phases.iter().any(|phase| phase.name == phase_name) {
                self.phases.extend(
                    Self::default_phases()
                        .into_iter()
                        .filter(|phase| phase.name == phase_name),
                );
            }
            return;
        }

        self.phases.retain(|phase| phase.name != phase_name);
    }

    pub async fn run(
        &mut self,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        pattern: Option<&str>,
        path: Option<&str>,
        llm: Option<&dyn LLMClient>,
        model: Option<&str>,
    ) -> WorkflowResult {
        self.run_with_registry(
            &AgentFactoryRegistry::with_builtins(),
            registry,
            memory,
            pattern,
            path,
            llm,
            model,
        )
        .await
    }

    pub async fn run_with_registry(
        &mut self,
        factory_registry: &AgentFactoryRegistry,
        registry: &mut ToolRegistry,
        memory: &mut MemoryWorkspace,
        pattern: Option<&str>,
        path: Option<&str>,
        llm: Option<&dyn LLMClient>,
        model: Option<&str>,
    ) -> WorkflowResult {
        let start = Instant::now();
        self.status = status::RUNNING.to_string();

        let workspace = registry.workspace().to_path_buf();
        let mut context = AgentExecutionContext {
            workspace: &workspace,
            registry,
            memory,
            llm,
            model,
        };

        let mut phases = Vec::new();
        for phase in self.phases.clone() {
            let phase_start = Instant::now();
            let request = phase.into_request(&self.session_id, pattern, path);
            let agent_id = request.agent_id.clone();
            let phase_name = phase.name.clone();

            let result = match execute_agent_with_context(factory_registry, &mut context, request).await
            {
                Ok(result) => result,
                Err(error) => lab_core::types::AgentResult::fail(error.to_string(), None),
            };

            let phase_result = PhaseResult {
                name: phase_name.clone(),
                success: result.success,
                data: result.data.clone(),
                duration_secs: phase_start.elapsed().as_secs_f64(),
                agent_id,
                error: result.error.clone().unwrap_or_default(),
            };
            let failed = !phase_result.success;
            phases.push(phase_result);

            if failed {
                if !phase.continue_on_failure {
                    self.status = status::FAILED.to_string();
                    return self.finish(
                        phases,
                        start.elapsed().as_secs_f64(),
                        format!("{phase_name} phase failed"),
                    );
                }
                warn!("Collaborator: {phase_name} phase failed — continuing");
            } else {
                info!(
                    "Collaborator: {phase_name} phase completed ({:.1}s)",
                    phases.last().map(|result| result.duration_secs).unwrap_or_default()
                );
            }
        }

        self.status = status::COMPLETED.to_string();
        self.finish(phases, start.elapsed().as_secs_f64(), String::new())
    }

    fn finish(&self, phases: Vec<PhaseResult>, total_secs: f64, error: String) -> WorkflowResult {
        if error.is_empty() {
            info!(
                "Collaborator: pipeline completed in {:.1}s ({} phases)",
                total_secs,
                phases.len()
            );
        }

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
        let collaborator = MultiAgentCollaborator::new("test-session");
        assert_eq!(collaborator.status, status::IDLE);
        assert_eq!(collaborator.phases().len(), 4);
    }

    #[test]
    fn collaborator_flags() {
        let collaborator = MultiAgentCollaborator::new("s1")
            .enable_review(false)
            .enable_code_generation(false);

        let phase_names: Vec<&str> = collaborator
            .phases()
            .iter()
            .map(|phase| phase.name.as_str())
            .collect();
        assert_eq!(phase_names, vec!["research", "summary"]);
    }

    #[test]
    fn workflow_result_success() {
        let result = WorkflowResult {
            status: status::COMPLETED.to_string(),
            phases: vec![PhaseResult {
                name: "research".into(),
                success: true,
                data: serde_json::json!({}),
                duration_secs: 1.0,
                agent_id: "r1".into(),
                error: String::new(),
            }],
            total_duration_secs: 1.0,
            error: String::new(),
        };
        assert!(result.success());
    }
}
