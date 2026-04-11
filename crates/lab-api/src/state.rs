use lab_core::{EventBus, LabConfig, ResearchLab};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state accessible by all route handlers.
pub struct AppState {
    pub lab: RwLock<ResearchLab>,
    /// The lab's event bus, held directly as an `Arc` so WebSocket handlers
    /// can subscribe without locking the lab.
    pub events: Arc<EventBus>,
}

impl AppState {
    pub async fn new(config: LabConfig) -> Self {
        let lab = ResearchLab::new(config);
        lab.start().await.expect("Failed to start lab");
        lab.restore_sessions().await;

        let events = lab.event_bus_arc();

        Self {
            lab: RwLock::new(lab),
            events,
        }
    }
}
