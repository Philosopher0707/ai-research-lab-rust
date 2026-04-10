use lab_core::{LabConfig, LabEvent, ResearchLab};
use tokio::sync::{broadcast, RwLock};

/// Shared application state accessible by all route handlers.
pub struct AppState {
    pub lab: RwLock<ResearchLab>,
    /// Broadcast channel — every EventBus event is forwarded here.
    /// WebSocket handlers subscribe to this for real-time streaming.
    pub event_tx: broadcast::Sender<String>,
}

impl AppState {
    pub async fn new(config: LabConfig) -> Self {
        let mut lab = ResearchLab::new(config);
        lab.start().await.expect("Failed to start lab");
        lab.restore_sessions();

        let (event_tx, _) = broadcast::channel::<String>(512);
        let tx = event_tx.clone();

        lab.event_bus_mut().subscribe_all(move |event: LabEvent| {
            let message = serde_json::json!({
                "type": event.event_type,
                "data": event.data,
                "source": event.source,
            })
            .to_string();
            let _ = tx.send(message);
        });

        Self {
            lab: RwLock::new(lab),
            event_tx,
        }
    }
}
