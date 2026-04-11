//! Domain services — each wraps a subsystem behind the minimal lock
//! needed to convert `&mut self` APIs into concurrent `&self` access.
//!
//! The services are consumed by `LabRuntime`, which holds each one in an
//! `Arc` so that `ResearchLab` (a thin `Arc<LabRuntime>` shim) and API
//! handlers can share them without holding a coarse write lock on the lab.

pub mod llm_service;
pub mod memory_service;
pub mod session_service;
pub mod tool_service;

pub use llm_service::LlmService;
pub use memory_service::MemoryService;
pub use session_service::SessionService;
pub use tool_service::ToolService;
