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

pub mod base;
pub mod coder;
pub mod collaborator;
pub mod communication;
pub mod llm_agents;
pub mod researcher;
pub mod reviewer;
pub mod summarizer;

pub use base::AgentImpl;
pub use coder::CoderAgent;
pub use collaborator::MultiAgentCollaborator;
pub use communication::{AgentCommunicator, AgentMessage, MessageType};
pub use researcher::ResearcherAgent;
pub use reviewer::ReviewerAgent;
pub use summarizer::SummarizerAgent;
