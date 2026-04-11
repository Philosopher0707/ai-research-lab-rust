//! Agent framework — concrete research agents for the AI Research Lab.
//!
//! Mirrors agents/ in the Python project:
//! - base.rs: AgentImpl (implements Agent trait)
//! - researcher.rs: discovers and analyses codebase structure
//! - coder.rs: generates/edits code files
//! - reviewer.rs: static analysis and code review
//! - summarizer.rs: creates consolidated reports from memory
//! - communication.rs: inter-agent messaging (mailbox + communicator)
//! - collaborator.rs: multi-agent pipeline orchestrator
//! - llm_agents.rs: LLM-enhanced agents with heuristic fallback

pub mod autonomous_improvement;
pub mod base;
pub mod coder;
pub mod collaborator;
pub mod communication;
pub mod improvement;
pub mod improvement_planner;
pub mod llm_agents;
pub mod researcher;
pub mod reviewer;
pub mod runtime;
pub mod self_edit;
pub mod summarizer;

pub use autonomous_improvement::{
    AutonomousImprovementPipeline, ImprovementAttempt, ImprovementConfig, ImprovementReport,
};
pub use base::AgentImpl;
pub use coder::CoderAgent;
pub use collaborator::{CollaborationPhase, MultiAgentCollaborator};
pub use communication::{AgentCommunicator, AgentMessage, MessageType};
pub use improvement_planner::ImprovementCandidate;
pub use researcher::ResearcherAgent;
pub use reviewer::ReviewerAgent;
pub use runtime::{
    execute_agent, execute_agent_with_context, execute_agent_with_registry, supported_agent_types,
    AgentExecutionContext, AgentExecutionRequest, AgentFactory, AgentFactoryRegistry, AgentPlugin,
};
pub use self_edit::SelfEditAgent;
pub use summarizer::SummarizerAgent;

/// Shared one-shot LLM helper used by all agents.
/// Sends an optional system prompt + user prompt and returns the response, or `None` on error.
pub async fn ask_agent_llm(
    llm: &dyn lab_core::llm::LLMClient,
    model: &str,
    system: Option<&str>,
    prompt: String,
    temperature: f64,
    max_tokens: u32,
) -> Option<String> {
    let mut messages = Vec::new();
    if let Some(sys) = system {
        messages.push(lab_core::llm::ChatMessage::system(sys));
    }
    messages.push(lab_core::llm::ChatMessage::user(prompt));

    match llm.chat(messages, model, temperature, max_tokens).await {
        Ok(resp) => Some(resp.content),
        Err(e) => {
            tracing::warn!("LLM call failed: {}", e);
            None
        }
    }
}
