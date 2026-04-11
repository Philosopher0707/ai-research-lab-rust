//! ToolService — wraps `ToolRegistry` and `PermissionEngine` behind
//! separate async `Mutex`es, converting their `&mut self` requirements
//! into `&self`-compatible concurrent access.

use lab_permissions::PermissionEngine;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use tokio::sync::Mutex;

pub struct ToolService {
    registry: Mutex<ToolRegistry>,
    permission: Mutex<PermissionEngine>,
}

impl ToolService {
    pub fn new(registry: ToolRegistry, permission: PermissionEngine) -> Self {
        Self {
            registry: Mutex::new(registry),
            permission: Mutex::new(permission),
        }
    }

    // ─── Tool registry read access ────────────────────────────────

    /// Call `f` with a shared reference to the registry (no `&mut` needed).
    ///
    /// Used for read-only queries (`get`, `list_tools`, `get_stats`).
    pub async fn with_registry<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&ToolRegistry) -> R,
    {
        f(&*self.registry.lock().await)
    }

    /// Check whether a tool with `name` is registered.
    pub async fn get(&self, name: &str) -> bool {
        self.registry.lock().await.get(name).is_some()
    }

    /// Return the full tool list as JSON metadata.
    pub async fn list_tools(&self) -> Vec<serde_json::Value> {
        self.registry.lock().await.list_tools()
    }

    // ─── Permission check ──────────────────────────────────────────

    /// Returns the raw `allowed`/`reason` map from `PermissionEngine::check`.
    pub async fn check_permission(
        &self,
        agent_id: &str,
        tool_name: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> HashMap<String, serde_json::Value> {
        self.permission
            .lock()
            .await
            .check(agent_id, tool_name, params)
            .await
    }

    // ─── Tool execution ────────────────────────────────────────────

    pub async fn execute(
        &self,
        tool_name: &str,
        params: HashMap<String, serde_json::Value>,
        agent_id: Option<&str>,
    ) -> serde_json::Value {
        if let Some(aid) = agent_id {
            let perm = self.check_permission(aid, tool_name, &params).await;
            let allowed = perm
                .get("allowed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !allowed {
                return serde_json::json!({
                    "success": false,
                    "error": "permission_denied",
                    "message": perm.get("reason").and_then(|v| v.as_str()).unwrap_or("denied"),
                });
            }
        }
        self.registry.lock().await.execute(tool_name, &params).await
    }
}
