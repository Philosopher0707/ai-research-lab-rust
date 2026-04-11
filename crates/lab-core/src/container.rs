use crate::config::LabConfig;
use crate::events::EventBus;
use crate::llm::{create_client, LLMClient};
use crate::sessions::SessionStore;
use lab_memory::MemoryWorkspace;
use lab_permissions::{PermissionEngine, PermissionPolicy};
use lab_tools::{ToolPlugin, ToolRegistry};
use std::sync::Arc;

enum LlmClientMode {
    Auto,
    Disabled,
    Provided(Box<dyn LLMClient>),
}

/// Fully constructed runtime dependencies for a `ResearchLab`.
pub struct LabContainer {
    pub config: LabConfig,
    pub memory: MemoryWorkspace,
    pub tool_registry: ToolRegistry,
    pub permission_engine: PermissionEngine,
    pub session_store: SessionStore,
    /// Shared event bus — wrapped in `Arc` so the API server can hold its own
    /// clone without needing to lock the lab to subscribe or emit events.
    pub event_bus: Arc<EventBus>,
    pub llm_client: Option<Box<dyn LLMClient>>,
}

/// Builder for dependency-injected lab services.
pub struct LabContainerBuilder {
    config: LabConfig,
    memory: Option<MemoryWorkspace>,
    tool_registry: Option<ToolRegistry>,
    permission_engine: Option<PermissionEngine>,
    session_store: Option<SessionStore>,
    event_bus: Option<Arc<EventBus>>,
    llm_mode: LlmClientMode,
    tool_plugins: Vec<Box<dyn ToolPlugin>>,
    register_builtin_tools: bool,
}

impl LabContainerBuilder {
    pub fn new(config: LabConfig) -> Self {
        Self {
            config,
            memory: None,
            tool_registry: None,
            permission_engine: None,
            session_store: None,
            event_bus: None,
            llm_mode: LlmClientMode::Auto,
            tool_plugins: Vec::new(),
            register_builtin_tools: true,
        }
    }

    pub fn with_memory(mut self, memory: MemoryWorkspace) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn with_tool_registry(mut self, tool_registry: ToolRegistry) -> Self {
        self.tool_registry = Some(tool_registry);
        self
    }

    pub fn with_permission_engine(mut self, permission_engine: PermissionEngine) -> Self {
        self.permission_engine = Some(permission_engine);
        self
    }

    pub fn with_session_store(mut self, session_store: SessionStore) -> Self {
        self.session_store = Some(session_store);
        self
    }

    pub fn with_event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    pub fn with_llm_client(mut self, llm_client: Box<dyn LLMClient>) -> Self {
        self.llm_mode = LlmClientMode::Provided(llm_client);
        self
    }

    pub fn without_llm(mut self) -> Self {
        self.llm_mode = LlmClientMode::Disabled;
        self
    }

    pub fn with_tool_plugin(mut self, plugin: Box<dyn ToolPlugin>) -> Self {
        self.tool_plugins.push(plugin);
        self
    }

    pub fn without_builtin_tools(mut self) -> Self {
        self.register_builtin_tools = false;
        self
    }

    pub fn build(self) -> LabContainer {
        let config = self.config;
        let memory = self
            .memory
            .unwrap_or_else(|| MemoryWorkspace::new(config.full_path(&config.memory_dir)));

        let mut tool_registry = self
            .tool_registry
            .unwrap_or_else(|| ToolRegistry::new(config.workspace.clone()));
        if self.register_builtin_tools {
            tool_registry.register_builtins();
        }
        for plugin in self.tool_plugins {
            tool_registry.register_plugin(plugin);
        }

        let permission_engine = self
            .permission_engine
            .unwrap_or_else(|| PermissionEngine::new(PermissionPolicy::default()));
        let session_store = self
            .session_store
            .unwrap_or_else(|| SessionStore::new(config.full_path(&config.sessions_dir)));
        let event_bus = self
            .event_bus
            .unwrap_or_else(|| Arc::new(EventBus::new(512, 1000)));

        let llm_client = match self.llm_mode {
            LlmClientMode::Auto if config.llm_configured() && !config.provider.is_empty() => {
                Some(create_client(
                    &config.provider,
                    &config.api_key,
                    &config.model,
                    &config.base_url,
                ))
            }
            LlmClientMode::Auto | LlmClientMode::Disabled => None,
            LlmClientMode::Provided(client) => Some(client),
        };

        LabContainer {
            config,
            memory,
            tool_registry,
            permission_engine,
            session_store,
            event_bus,
            llm_client,
        }
    }
}
