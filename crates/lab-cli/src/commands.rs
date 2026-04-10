use crate::ui::{
    llm_setup_hint, print_chat_header, print_error, print_help_menu, print_meta_row, print_rule,
    print_success, print_warning, provider_display_name, with_spinner,
};
use colored::Colorize;
use lab_core::config::AgentProfile;
use lab_core::llm::ChatMessage;
use lab_core::{LabConfig, ResearchLab};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) async fn cmd_chat() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let mut lab = ResearchLab::new(config.clone());
    with_spinner("Booting workspace", lab.start()).await?;
    print_chat_header(&config, lab.has_llm());

    let project_ctx = build_project_context(&config);
    let system_prompt = format!(
        "You are an AI assistant for the AI Research Lab — a Rust-based multi-agent research system.\n\
         You can help with architecture questions, code reviews, Rust patterns, and debugging.\n\
         Be concise and direct. Use code blocks when showing code.\n\n\
         {project_ctx}"
    );

    let mut history = vec![ChatMessage::system(&system_prompt)];

    loop {
        print!("{}", "lab › ".cyan().bold());
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).unwrap_or(0) == 0 {
            break;
        }
        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        match input.as_str() {
            "/exit" | "/quit" | "/q" => {
                println!();
                println!("{}", "Session closed.".dimmed());
                break;
            }
            "/help" | "/?" => {
                print_help_menu();
                continue;
            }
            "/status" => {
                let _ = cmd_status().await;
                println!();
                continue;
            }
            "/pipeline" => {
                let _ = cmd_pipeline_run("**/*.rs", None, false, true).await;
                println!();
                continue;
            }
            "/clear" => {
                history.truncate(1);
                print_success("Conversation cleared for this session.");
                println!();
                continue;
            }
            _ => {}
        }

        if !lab.has_llm() {
            print_warning(format!("LLM unavailable — {}.", llm_setup_hint(&config)));
            println!();
            continue;
        }

        history.push(ChatMessage::user(&input));

        match with_spinner("Thinking", lab.ask_llm_messages(history.clone(), 0.7, 1024)).await {
            Some(answer) => {
                println!();
                println!("{}", "assistant".bright_black().bold());
                println!("{}", answer.trim());
                println!();
                history.push(ChatMessage::assistant(&answer));

                if history.len() > 22 {
                    let system = history[0].clone();
                    history.drain(1..3);
                    history[0] = system;
                }
            }
            None => {
                print_error("No response from the configured LLM.");
                history.pop();
                println!();
            }
        }
    }

    lab.shutdown().await?;
    Ok(())
}

pub(crate) fn cmd_init(workspace: &str) -> anyhow::Result<()> {
    let workspace = if workspace != "." {
        let path = PathBuf::from(workspace);
        std::fs::create_dir_all(&path)?;
        path
    } else {
        std::env::current_dir()?
    };
    let config = LabConfig::with_workspace(workspace.clone());

    println!();
    println!("{}", "AI Research Lab".bright_white().bold());
    println!(
        "{}",
        "Workspace initialized and ready for CLI sessions.".dimmed()
    );
    print_rule();
    print_meta_row("Workspace", workspace.display());
    print_meta_row("Memory", config.full_path(&config.memory_dir).display());
    print_meta_row("Sessions", config.full_path(&config.sessions_dir).display());
    print_meta_row("Outputs", config.full_path(&config.outputs_dir).display());
    print_rule();
    println!(
        "  {} {}",
        "Next".bright_black(),
        "Run `lab` to open chat or `lab setup` to configure an API.".dimmed()
    );
    Ok(())
}

pub(crate) async fn cmd_pipeline_run(
    pattern: &str,
    _path: Option<&str>,
    no_review: bool,
    no_code: bool,
) -> anyhow::Result<()> {
    let config = LabConfig::default();

    println!();
    println!("{}", "Pipeline Run".bright_white().bold());
    println!(
        "{}",
        "Research pipeline execution for the current workspace.".dimmed()
    );
    print_rule();
    print_meta_row("Workspace", config.workspace.display());
    print_meta_row("Pattern", pattern);
    print_meta_row("Mode", if no_review { "Quick" } else { "Full" });
    print_meta_row(
        "LLM",
        if config.llm_configured() {
            "Enabled".green().bold().to_string()
        } else {
            "Heuristic only".yellow().bold().to_string()
        },
    );
    print_rule();

    let session_id = format!("pipeline-{}", &uuid::Uuid::new_v4().to_string()[..6]);
    let (request, output_path) = build_cli_pipeline_request(&config, pattern, no_review, no_code);
    let result = with_spinner(
        "Running pipeline",
        lab_pipelines::run_pipeline_with_config(&config, request, session_id),
    )
    .await;

    print_pipeline_results(&result, &output_path);
    Ok(())
}

pub(crate) async fn cmd_ask_once(question: &str) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config.clone());
    if !lab.has_llm() {
        print_error(format!("LLM unavailable — {}.", llm_setup_hint(&config)));
        return Ok(());
    }

    let project_ctx = build_project_context(&config);
    let messages = vec![
        ChatMessage::system("You are an AI code assistant. Answer concisely about the project."),
        ChatMessage::user(format!("{project_ctx}\n\nQ: {question}")),
    ];

    match with_spinner("Thinking", lab.ask_llm_messages(messages, 0.7, 1024)).await {
        Some(text) => {
            println!("{}", "assistant".bright_black().bold());
            println!("{text}");
        }
        None => print_error("No response from the configured LLM."),
    }
    Ok(())
}

pub(crate) async fn cmd_serve(port: u16) -> anyhow::Result<()> {
    let config = LabConfig::default();
    config.ensure_directories();
    let state = Arc::new(lab_api::AppState::new(config).await);
    let router = lab_api::create_router(state.clone());
    let addr = format!("0.0.0.0:{port}");
    let (provider, model, llm_ready) = {
        let lab = state.lab.read().await;
        (
            provider_display_name(&lab.config.provider).to_string(),
            lab.config.model.clone(),
            lab.has_llm(),
        )
    };

    println!();
    println!("{}", "API Server".bright_white().bold());
    println!(
        "{}",
        "REST and WebSocket endpoints for the research lab.".dimmed()
    );
    print_rule();
    print_meta_row("Listen", addr.as_str().cyan().bold());
    print_meta_row("Provider", provider);
    print_meta_row("Model", model);
    print_meta_row(
        "LLM",
        if llm_ready {
            "Ready".green().bold().to_string()
        } else {
            "Unavailable".yellow().bold().to_string()
        },
    );
    print_rule();
    println!("  GET  /health      {}", "Health check".dimmed());
    println!("  POST /sessions    {}", "Create a session".dimmed());
    println!("  POST /agents/run  {}", "Run an agent".dimmed());
    println!("  POST /ask         {}", "Ask the configured LLM".dimmed());
    println!(
        "  WS   /events      {}",
        "Stream real-time lab events".dimmed()
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

pub(crate) fn cmd_tools() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config);
    let tools = lab.tools().list_tools();

    println!(
        "{}",
        format!("═══ {} tools registered:", tools.len())
            .bold()
            .yellow()
    );
    for tool in &tools {
        let name = tool
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("?");
        let description = tool
            .get("description")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let category = tool
            .get("category")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        println!(
            "  {} — {} [{}]",
            name.bold(),
            description,
            category.dimmed()
        );
    }
    Ok(())
}

pub(crate) async fn cmd_list() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let sessions_dir = config.full_path(&config.sessions_dir);
    let count = session_record_count(&sessions_dir);
    println!(
        "{}",
        format!(
            "═══ {count} session record(s) in {}",
            sessions_dir.display()
        )
        .bold()
        .cyan()
    );
    Ok(())
}

pub(crate) fn cmd_clear(yes: bool) -> anyhow::Result<()> {
    if yes {
        let config = LabConfig::default();
        let memory_dir = config.full_path(&config.memory_dir);
        if memory_dir.exists() {
            std::fs::remove_dir_all(&memory_dir)?;
        }
        std::fs::create_dir_all(&memory_dir)?;
        println!("{} Memory cleared.", "✓".green().bold());
    } else {
        println!("{} Run `lab clear --yes` to confirm.", "⚠".yellow().bold());
    }
    Ok(())
}

pub(crate) async fn cmd_status() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config.clone());
    let sessions_dir = config.full_path(&config.sessions_dir);

    println!();
    println!("{}", "AI Research Lab Status".bright_white().bold());
    println!(
        "{}",
        "Runtime view of the current workspace and provider configuration.".dimmed()
    );
    print_rule();
    print_meta_row("Workspace", lab.config.workspace.display());
    print_meta_row("Provider", provider_display_name(&lab.config.provider));
    print_meta_row("Model", lab.config.model.as_str());
    print_meta_row("Tools", lab.tools().list_tools().len());
    print_meta_row("Sessions", session_record_count(&sessions_dir));
    print_meta_row("Memory", lab.memory().entry_count());
    print_meta_row(
        "LLM",
        if lab.has_llm() {
            "Ready".green().bold().to_string()
        } else {
            "Needs setup".yellow().bold().to_string()
        },
    );
    if !lab.has_llm() {
        print_meta_row("Hint", llm_setup_hint(&lab.config).yellow());
    }
    Ok(())
}

pub(crate) async fn cmd_agent(agent_type: &str, pattern: &str) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let workspace = config.workspace.clone();

    let session_id = format!("agent-{}", &uuid::Uuid::new_v4().to_string()[..6]);
    let profile = AgentProfile::default();
    let agent_id = format!("{agent_type}-{}", &uuid::Uuid::new_v4().to_string()[..6]);

    println!();
    println!("{}", "Agent Run".bright_white().bold());
    println!(
        "{}",
        "Execute a focused agent against the current workspace.".dimmed()
    );
    print_rule();
    print_meta_row("Agent", agent_type);
    print_meta_row("Pattern", pattern);
    print_meta_row("Workspace", workspace.display());
    print_rule();

    let request = lab_agents::AgentExecutionRequest {
        agent_type: agent_type.to_string(),
        agent_id,
        session_id,
        task: match agent_type {
            "reviewer" => "review code quality",
            "summarizer" => "generate summary",
            _ => "analyze workspace",
        }
        .to_string(),
        pattern: Some(pattern.to_string()),
        path: None,
        output_path: Some(
            config
                .full_path(&config.outputs_dir)
                .join("summary.md")
                .to_string_lossy()
                .to_string(),
        ),
        content: None,
        template: None,
        profile,
    };
    let result =
        match with_spinner("Running agent", lab_agents::execute_agent(&config, request)).await {
            Ok(result) => result,
            Err(error) => {
                print_error(error);
                return Ok(());
            }
        };

    if result.success {
        println!(
            "{}",
            serde_json::to_string_pretty(&result.data).unwrap_or_default()
        );
    } else {
        print_error(format!("Agent failed: {:?}", result.error));
    }
    Ok(())
}

fn build_project_context(config: &LabConfig) -> String {
    let mut context = String::new();
    for file_name in ["Cargo.toml", "RUNTIME_MAP.md", "DESIGN.md", "README.md"] {
        let path = config.workspace.join(file_name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            let snippet = if content.len() > 1500 {
                &content[..1500]
            } else {
                &content
            };
            context.push_str(&format!("=== {file_name} ===\n{snippet}\n\n"));
        }
    }

    if context.is_empty() {
        "No project files found in workspace.".to_string()
    } else {
        context
    }
}

fn build_cli_pipeline_request(
    config: &LabConfig,
    pattern: &str,
    no_review: bool,
    no_code: bool,
) -> (lab_pipelines::ExecutionRequest, PathBuf) {
    let output_path = config
        .full_path(&config.outputs_dir)
        .join("pipeline-report.md");
    let request = lab_pipelines::ExecutionRequest::from_cli(
        "cli-pipeline",
        pattern,
        no_review,
        no_code,
        Some(output_path.clone()),
    );
    (request, output_path)
}

fn print_pipeline_results(result: &lab_pipelines::PipelineResult, output_path: &Path) {
    println!();
    println!("{}", "Pipeline Results".bright_white().bold());
    print_rule();
    println!(
        "{} {}",
        format!("{:<12}", "Status").bright_black(),
        match result.status.as_str() {
            "completed" => "✅ Completed".green().to_string(),
            "failed" => "❌ Failed".red().to_string(),
            _ => "⚠ Partial".yellow().to_string(),
        }
    );
    for stage in &result.stage_results {
        let icon = if matches!(stage.status, lab_pipelines::StageStatus::Completed) {
            "✅"
        } else {
            "❌"
        };
        println!(
            "  {} {} ({:.1}s)",
            icon,
            stage.name.bold(),
            stage.duration_secs
        );
        if let Some(error) = &stage.error {
            println!("    {}", error.dimmed());
        }
    }
    println!();
    print_meta_row("Total", format!("{:.1}s", result.total_duration_secs));
    if output_path.exists() {
        print_meta_row("Report", output_path.display());
    }
}

fn session_record_count(path: &Path) -> usize {
    if path.exists() {
        std::fs::read_dir(path)
            .map(|entries| entries.count())
            .unwrap_or(0)
    } else {
        0
    }
}
