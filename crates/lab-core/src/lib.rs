//! Lab Core — Configuration, domain types, research engine, events,
//! scheduler, sessions, workflows, and LLM integration.
//!
//! This is the heart of the AI Research Lab, tying together memory,
//! tools, and permissions into a cohesive agent-driven research system.

pub mod config;
pub mod engine;
pub mod errors;
pub mod events;
pub mod llm;
pub mod scheduler;
pub mod sessions;
pub mod types;
pub mod workflows;

// Re-exports for convenience
pub use config::*;
pub use engine::ResearchLab;
pub use errors::{LabError, Result};
pub use events::*;
pub use llm::*;
pub use scheduler::*;
pub use sessions::*;
pub use types::*;
pub use workflows::*;
