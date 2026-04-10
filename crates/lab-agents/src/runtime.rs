use crate::{CoderAgent, ResearcherAgent, ReviewerAgent, SummarizerAgent};
use lab_core::config::AgentProfile;
use lab_core::types::AgentResult;
use lab_core::LabConfig;
use lab_memory::MemoryWorkspace;
use lab_tools::ToolRegistry;

struct AgentRuntime {
    registry: ToolRegistry,
    memory: MemoryWorkspace,
    llm: Option<Box<dyn lab_core::llm::LLMClient>>,
}

impl AgentRuntime {
    fn from_config(config: &LabConfig) -> Self {
        let workspace = config.workspace.clone();
        let memory_dir = config.full_path(&config.memory_dir);
        let mut registry = ToolRegistry::new(workspace);
        registry.register_builtins();

        let llm = if config.llm_configured() && !config.provider.is_empty() {
            Some(lab_core::llm::create_client(
                &config.provider,
                &config.api_key,
                &config.model,
                &config.base_url,
            ))
        } else {
            None
        };

        Self {
            registry,
            memory: MemoryWorkspace::new(memory_dir),
            llm,
        }
    }
}

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
}

pub fn supported_agent_types() -> &'static str {
    "researcher | reviewer | coder | summarizer"
}

pub async fn execute_agent(
    config: &LabConfig,
    request: AgentExecutionRequest,
) -> anyhow::Result<AgentResult> {
    let mut runtime = AgentRuntime::from_config(config);
    let llm_ref = runtime.llm.as_deref();
    let model = llm_ref.map(|_| config.model.as_str());
    let workspace = config.workspace.clone();

    let AgentExecutionRequest {
        agent_type,
        agent_id,
        session_id,
        task,
        pattern,
        path,
        output_path,
        content,
        template,
        profile,
    } = request;

    let result = match agent_type.as_str() {
        "researcher" => {
            let mut agent =
                ResearcherAgent::new(agent_id, session_id, profile.clone(), workspace.clone());
            agent
                .execute(
                    &mut runtime.registry,
                    &mut runtime.memory,
                    &task,
                    pattern.as_deref(),
                    path.as_deref(),
                    None,
                    llm_ref,
                    model,
                )
                .await
        }
        "reviewer" => {
            let mut agent =
                ReviewerAgent::new(agent_id, session_id, profile.clone(), workspace.clone());
            agent
                .execute(
                    &mut runtime.registry,
                    &mut runtime.memory,
                    &task,
                    pattern.as_deref(),
                    path.as_deref(),
                    None,
                    llm_ref,
                    model,
                )
                .await
        }
        "coder" => {
            let mut agent = CoderAgent::new(agent_id, session_id, profile, workspace);
            agent
                .execute(
                    &mut runtime.registry,
                    &mut runtime.memory,
                    &task,
                    path.as_deref(),
                    content.as_deref(),
                    template.as_deref(),
                    llm_ref,
                    model,
                )
                .await
        }
        "summarizer" => {
            let mut agent = SummarizerAgent::new(agent_id, session_id, profile, workspace);
            agent
                .execute(
                    &mut runtime.registry,
                    &mut runtime.memory,
                    &task,
                    output_path.as_deref(),
                    llm_ref,
                    model,
                )
                .await
        }
        unknown => anyhow::bail!(
            "Unknown agent type: {unknown}. Use: {}",
            supported_agent_types()
        ),
    };

    Ok(result)
}
