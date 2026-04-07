//! Lab permissions — RBAC engine
//!
//! Granular role-based access control with restriction levels,
//! tool categorization, and audit trail. Mirrors `permissions/rbac.py`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

// ─── Restriction Levels ─────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestrictionLevel {
    FullRestrict,
    ReadOnly,
    SafeTools,
    FullAccess,
}

impl Default for RestrictionLevel {
    fn default() -> Self {
        Self::SafeTools
    }
}

/// Map from config-level strings to RestrictionLevel
pub fn resolve_restriction(raw: &str) -> RestrictionLevel {
    match raw {
        "read" => RestrictionLevel::ReadOnly,
        "standard" => RestrictionLevel::SafeTools,
        "write" | "admin" | "super-admin" => RestrictionLevel::FullAccess,
        _ => RestrictionLevel::SafeTools,
    }
}

// ─── Tool Categories ────────────────────────────────────

/// Category a tool belongs to
pub fn tool_category(tool_name: &str) -> &'static str {
    match tool_name {
        "read_file" | "glob_search" | "grep_search" => "read",
        "write_file" | "edit_file" => "write",
        "bash" => "system",
        "web_search" | "web_fetch" => "network",
        _ => "custom",
    }
}

pub const SAFE_TOOLS: &[&str] = &[
    "read_file", "glob_search", "grep_search", "LSP", "ToolSearch",
];

// ─── Policy ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPolicy {
    pub default_restriction_level: RestrictionLevel,
    pub agent_policies: HashMap<String, RestrictionLevel>,
    pub allowed_tools: HashMap<String, Vec<String>>,
    pub blocked_tools: Vec<String>,
    pub audit_all_operations: bool,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self {
            default_restriction_level: RestrictionLevel::SafeTools,
            agent_policies: HashMap::new(),
            allowed_tools: HashMap::new(),
            blocked_tools: vec!["TestingPermission".to_string()],
            audit_all_operations: true,
        }
    }
}

impl PermissionPolicy {
    pub fn get_restriction_level(&self, agent_id: Option<&str>) -> RestrictionLevel {
        if let Some(id) = agent_id {
            if let Some(level) = self.agent_policies.get(id) {
                return *level;
            }
        }
        self.default_restriction_level
    }
}

// ─── Audit Record ───────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PermissionCheck {
    pub agent_id: String,
    pub tool_name: String,
    pub allowed: bool,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: f64,
}

// ─── Permission Engine ──────────────────────────────────

pub struct PermissionEngine {
    pub policy: PermissionPolicy,
    audit_log: Vec<PermissionCheck>,
    denied_count: u64,
    approved_count: u64,
}

impl PermissionEngine {
    pub fn new(policy: PermissionPolicy) -> Self {
        Self {
            policy,
            audit_log: Vec::new(),
            denied_count: 0,
            approved_count: 0,
        }
    }

    pub async fn check(
        &mut self,
        agent_id: &str,
        tool_name: &str,
        _params: &HashMap<String, serde_json::Value>,
    ) -> HashMap<String, serde_json::Value> {
        let start = Instant::now();
        let level = self.policy.get_restriction_level(Some(agent_id));
        let category = tool_category(tool_name);

        // Global blocklist
        if self.policy.blocked_tools.iter().any(|t| t == tool_name) {
            let reason = format!("Tool '{tool_name}' is globally blocked");
            return self.record(agent_id, tool_name, false, &reason, start);
        }

        let result = match level {
            RestrictionLevel::FullRestrict => {
                (false, "Full restriction mode: all tools denied".to_string())
            }
            RestrictionLevel::ReadOnly => {
                if category == "read" {
                    (true, format!("Read-only mode: '{tool_name}' is a read tool"))
                } else {
                    (false, format!("Read-only mode: '{tool_name}' is not a read tool"))
                }
            }
            RestrictionLevel::SafeTools => {
                if SAFE_TOOLS.contains(&tool_name) {
                    (true, format!("Safe tools mode: '{tool_name}' is allowed"))
                } else if category == "write" {
                    if let Some(allowed) = self.policy.allowed_tools.get("write") {
                        if allowed.iter().any(|t| t == tool_name) {
                            return self.record(agent_id, tool_name, true,
                                "Explicitly allowed in safe mode", start);
                        }
                    }
                    (false, format!("Safe tools mode: write tool '{tool_name}' not explicitly allowed"))
                } else {
                    (false, format!("Safe tools mode: {category} tool '{tool_name}' denied"))
                }
            }
            RestrictionLevel::FullAccess => {
                (true, "Full access mode".to_string())
            }
        };

        self.record(agent_id, tool_name, result.0, &result.1, start)
    }

    fn record(
        &mut self,
        agent_id: &str,
        tool_name: &str,
        allowed: bool,
        reason: &str,
        start: Instant,
    ) -> HashMap<String, serde_json::Value> {
        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        if allowed {
            self.approved_count += 1;
        } else {
            self.denied_count += 1;
        }

        self.audit_log.push(PermissionCheck {
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            allowed,
            reason: reason.to_string(),
            timestamp: Utc::now(),
            duration_ms,
        });

        let mut map = HashMap::new();
        map.insert("allowed".to_string(), serde_json::Value::Bool(allowed));
        map.insert("reason".to_string(), serde_json::Value::String(reason.to_string()));
        map
    }

    pub fn audit_log(&self) -> &[PermissionCheck] {
        &self.audit_log
    }

    pub fn denied_count(&self) -> u64 {
        self.denied_count
    }

    pub fn approved_count(&self) -> u64 {
        self.approved_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_safe_tools_allowed() {
        // read_file is in SAFE_TOOLS
        assert!(SAFE_TOOLS.contains(&"read_file"));
    }

    #[test]
    fn test_resolve_restriction() {
        assert_eq!(resolve_restriction("read"), RestrictionLevel::ReadOnly);
        assert_eq!(resolve_restriction("standard"), RestrictionLevel::SafeTools);
        assert_eq!(resolve_restriction("admin"), RestrictionLevel::FullAccess);
    }
}
