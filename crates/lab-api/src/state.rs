use lab_core::{EventBus, LabConfig, ResearchLab};
use std::sync::Arc;

/// Shared application state accessible by all route handlers.
///
/// `ResearchLab` is an `Arc<LabRuntime>` shim — all methods are `&self`,
/// so no `RwLock` wrapper is needed here.
pub struct AppState {
    pub lab: ResearchLab,
    /// The lab's event bus, held directly as an `Arc` so WebSocket handlers
    /// can subscribe without touching the lab at all.
    pub events: Arc<EventBus>,
}

impl AppState {
    pub async fn new(config: LabConfig) -> Self {
        let lab = ResearchLab::new(config);
        lab.start().await.expect("Failed to start lab");
        lab.restore_sessions().await;

        let events = lab.event_bus_arc();

        Self { lab, events }
    }
}
