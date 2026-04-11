use lab_api::{create_router, AppState};
use lab_core::LabConfig;
use std::sync::Arc;

pub struct TestServer {
    pub state: Arc<AppState>,
    pub base_url: String,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl TestServer {
    pub async fn start(config: LabConfig) -> Self {
        let state = Arc::new(AppState::new(config).await);
        let router = create_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        Self {
            state,
            base_url: format!("http://{addr}"),
            shutdown_tx,
            server,
        }
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.server.await;
    }
}
