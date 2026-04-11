//! Lab Core — Configuration, domain types, research engine, events,
//! scheduler, sessions, workflows, and LLM integration.
//!
//! This is the heart of the AI Research Lab, tying together memory,
//! tools, and permissions into a cohesive agent-driven research system.

pub mod container;
pub mod config;
pub mod engine;
pub mod errors;
pub mod events;
pub mod llm;
pub mod providers;
pub mod runtime;
pub mod scheduler;
pub mod services;
pub mod sessions;
pub mod types;
pub mod workflows;

// Re-exports for convenience
pub use container::{LabContainer, LabContainerBuilder};
pub use config::*;
pub use engine::ResearchLab;
pub use errors::{LabError, LLMError, Result};
pub use events::*;
pub use llm::*;
pub use providers::{detect_from_env as detect_provider_from_env, find as find_provider, ProviderDef, PROVIDERS};
pub use runtime::LabRuntime;
pub use scheduler::*;
pub use services::{LlmService, MemoryService, SessionService, ToolService};
pub use sessions::*;
pub use types::*;
pub use workflows::*;
