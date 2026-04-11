use crate::{CoderAgent, ResearcherAgent, ReviewerAgent, SelfEditAgent, SummarizerAgent};
use anyhow::bail;
use async_trait::async_trait;
use lab_core::config::AgentProfile;
use lab_core::llm::LLMClient;
use lab_core::types::AgentResult;
use lab_core::{LabConfig, LabContainerBuilder};
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct AgentRuntime {
    workspace: PathBuf,
    registry: ToolRegistry,
    memory: MemoryWorkspace,
    llm: Option<Box<dyn LLMClient>>,
    model: String,
}

impl AgentRuntime {
    fn from_config(config: &LabConfig) -> Self {
        let container = LabContainerBuilder::new(config.clone()).build();
        Self {
            workspace: config.workspace.clone(),
            registry: container.tool_registry,
            memory: container.memory,
            llm: container.llm_client,
            model: config.model.clone(),
        }
    }
}

pub struct AgentExecutionContext<'a> {
    pub workspace: &'a Path,
    pub registry: &'a mut ToolRegistry,
    pub memory: &'a mut MemoryWorkspace,
    pub llm: Option<&'a dyn LLMClient>,
    pub model: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct AgentExecutionRequest {
    pub agent_type: String,
    pub agent_id: String,
    pub session_id: String,
    pub task: String,
    pub pattern: Option<String>,
    pub path: Option<String>,
    pub output_path: Option<String>,
    pub content: Option<String>,
    pub template: Option<String>,
    pub profile: AgentProfile,
    /// If true, self-edit agent shows proposed edits without applying them.
    pub dry_run: bool,
}

pub trait AgentPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn factories(&self) -> Vec<Box<dyn AgentFactory>>;
}

#[async_trait]
pub trait AgentFactory: Send + Sync {
    fn agent_type(&self) -> &str;
    async fn execute(
        &self,
        context: &mut AgentExecutionContext<'_>,
        request: &AgentExecutionRequest,
    ) -> anyhow::Result<AgentResult>;
}

pub struct AgentFactoryRegistry {
    factories: HashMap<String, Box<dyn AgentFactory>>,
    plugins: Vec<(String, String)>,
}

impl Default for AgentFactoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentFactoryRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            plugins: Vec::new(),
        }
    }

    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register_plugin(Box::new(BuiltinAgentPlugin));
        registry
    }

    pub fn register_factory(&mut self, factory: Box<dyn AgentFactory>) {
        self.factories
            .insert(factory.agent_type().to_string(), factory);
    }

    pub fn register_plugin(&mut self, plugin: Box<dyn AgentPlugin>) {
        let plugin_name = plugin.name().to_string();
        let plugin_version = plugin.version().to_string();
        for factory in plugin.factories() {
            self.register_factory(factory);
        }
        self.plugins.push((plugin_name, plugin_version));
    }

    pub fn get(&self, agent_type: &str) -> Option<&dyn AgentFactory> {
        self.factories
            .get(agent_type)
            .map(|factory| factory.as_ref())
    }

    pub fn supported_types(&self) -> Vec<String> {
        let mut types: Vec<String> = self.factories.keys().cloned().collect();
        types.sort();
        types
    }

    pub fn plugin_metadata(&self) -> &[(String, String)] {
        &self.plugins
    }
}

struct BuiltinAgentPlugin;

impl AgentPlugin for BuiltinAgentPlugin {
    fn name(&self) -> &str {
        "builtin-agents"
    }

    fn factories(&self) -> Vec<Box<dyn AgentFactory>> {
        vec![
            Box::new(ResearcherFactory),
            Box::new(ReviewerFactory),
            Box::new(CoderFactory),
            Box::new(SummarizerFactory),
            Box::new(SelfEditFactory),
        ]
    }
}

struct ResearcherFactory;

#[async_trait]
impl AgentFactory for ResearcherFactory {
    fn agent_type(&self) -> &str {
        "researcher"
    }

    async fn execute(
        &self,
        context: &mut AgentExecutionContext<'_>,
        request: &AgentExecutionRequest,
    ) -> anyhow::Result<AgentResult> {
        let mut agent = ResearcherAgent::new(
            request.agent_id.clone(),
            request.session_id.clone(),
            request.profile.clone(),
            context.workspace.to_path_buf(),
        );
        Ok(agent
            .execute(
                context.registry,
                context.memory,
                &request.task,
                request.pattern.as_deref(),
                request.path.as_deref(),
                None,
                context.llm,
                context.model,
            )
            .await)
    }
}

struct ReviewerFactory;

#[async_trait]
impl AgentFactory for ReviewerFactory {
    fn agent_type(&self) -> &str {
        "reviewer"
    }

    async fn execute(
        &self,
        context: &mut AgentExecutionContext<'_>,
        request: &AgentExecutionRequest,
    ) -> anyhow::Result<AgentResult> {
        let mut agent = ReviewerAgent::new(
            request.agent_id.clone(),
            request.session_id.clone(),
            request.profile.clone(),
            context.workspace.to_path_buf(),
        );
        Ok(agent
            .execute(
                context.registry,
                context.memory,
                &request.task,
                request.pattern.as_deref(),
                request.path.as_deref(),
                None,
                context.llm,
                context.model,
            )
            .await)
    }
}

struct CoderFactory;

#[async_trait]
impl AgentFactory for CoderFactory {
    fn agent_type(&self) -> &str {
        "coder"
    }

    async fn execute(
        &self,
        context: &mut AgentExecutionContext<'_>,
        request: &AgentExecutionRequest,
    ) -> anyhow::Result<AgentResult> {
        let mut agent = CoderAgent::new(
            request.agent_id.clone(),
            request.session_id.clone(),
            request.profile.clone(),
            context.workspace.to_path_buf(),
        );
        Ok(agent
            .execute(
                context.registry,
                context.memory,
                &request.task,
                request.path.as_deref(),
                request.content.as_deref(),
                request.template.as_deref(),
                context.llm,
                context.model,
            )
            .await)
    }
}

struct SummarizerFactory;

#[async_trait]
impl AgentFactory for SummarizerFactory {
    fn agent_type(&self) -> &str {
        "summarizer"
    }

    async fn execute(
        &self,
        context: &mut AgentExecutionContext<'_>,
        request: &AgentExecutionRequest,
    ) -> anyhow::Result<AgentResult> {
        let mut agent = SummarizerAgent::new(
            request.agent_id.clone(),
            request.session_id.clone(),
            request.profile.clone(),
            context.workspace.to_path_buf(),
        );
        Ok(agent
            .execute(
                context.registry,
                context.memory,
                &request.task,
                request.output_path.as_deref(),
                context.llm,
                context.model,
            )
            .await)
    }
}

struct SelfEditFactory;

#[async_trait]
impl AgentFactory for SelfEditFactory {
    fn agent_type(&self) -> &str {
        "self_edit"
    }

    async fn execute(
        &self,
        context: &mut AgentExecutionContext<'_>,
        request: &AgentExecutionRequest,
    ) -> anyhow::Result<AgentResult> {
        let file_path = request
            .path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("self_edit agent requires --path <file>"))?;
        let llm = context
            .llm
            .ok_or_else(|| anyhow::anyhow!("self_edit agent requires an LLM — run `lab setup`"))?;
        let model = context
            .model
            .ok_or_else(|| anyhow::anyhow!("self_edit agent: no model configured"))?;

        let mut agent = SelfEditAgent::new(
            request.agent_id.clone(),
            request.session_id.clone(),
            request.profile.clone(),
            context.workspace.to_path_buf(),
        );
        Ok(agent
            .execute(
                context.registry,
                context.memory,
                file_path,
                &request.task,
                request.dry_run,
                llm,
                model,
            )
            .await)
    }
}

pub fn supported_agent_types() -> String {
    AgentFactoryRegistry::with_builtins()
        .supported_types()
        .join(" | ")
}

pub async fn execute_agent_with_context(
    factory_registry: &AgentFactoryRegistry,
    context: &mut AgentExecutionContext<'_>,
    request: AgentExecutionRequest,
) -> anyhow::Result<AgentResult> {
    let agent_type = request.agent_type.clone();
    let Some(factory) = factory_registry.get(&agent_type) else {
        bail!(
            "Unknown agent type: {agent_type}. Use: {}",
            factory_registry.supported_types().join(" | ")
        );
    };

    factory.execute(context, &request).await
}

pub async fn execute_agent(
    config: &LabConfig,
    request: AgentExecutionRequest,
) -> anyhow::Result<AgentResult> {
    execute_agent_with_registry(config, &AgentFactoryRegistry::with_builtins(), request).await
}

pub async fn execute_agent_with_registry(
    config: &LabConfig,
    factory_registry: &AgentFactoryRegistry,
    request: AgentExecutionRequest,
) -> anyhow::Result<AgentResult> {
    let mut runtime = AgentRuntime::from_config(config);
    let workspace = runtime.workspace.clone();
    let llm = runtime.llm.as_deref();
    let model = llm.map(|_| runtime.model.as_str());
    let mut context = AgentExecutionContext {
        workspace: &workspace,
        registry: &mut runtime.registry,
        memory: &mut runtime.memory,
        llm,
        model,
    };

    execute_agent_with_context(factory_registry, &mut context, request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_reports_supported_types() {
        let registry = AgentFactoryRegistry::with_builtins();
        assert_eq!(
            registry.supported_types(),
            vec![
                "coder".to_string(),
                "researcher".to_string(),
                "reviewer".to_string(),
                "self_edit".to_string(),
                "summarizer".to_string(),
            ]
        );
        assert_eq!(registry.plugin_metadata().len(), 1);
    }
}
