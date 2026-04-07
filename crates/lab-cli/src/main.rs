//! Lab CLI — Command-line interface for the AI Research Lab.

use clap::{Parser, Subcommand};
use colored::Colorize;
use lab_agents::researcher::ResearcherAgent;
use lab_agents::reviewer::ReviewerAgent;
use lab_agents::summarizer::SummarizerAgent;
use lab_agents::collaborator::MultiAgentCollaborator;
use lab_api;
use lab_core::{LabConfig, ResearchLab};
use lab_reports::ReportGenerator;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "lab", about = "AI Research Lab — agentic research CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the lab workspace
    Init {
        /// Workspace directory
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
    /// Run a full research pipeline (Researcher → Reviewer → Summarizer)
    Pipeline {
        #[command(subcommand)]
        subcommand: PipelineCommands,
    },
    /// Ask a question about your codebase using LLM
    Ask {
        /// The question to ask
        question: String,
    },
    /// Serve the REST API + WebSocket server
    Serve {
        /// Port to bind to
        #[arg(short, long, default_value = "8000")]
        port: u16,
    },
    /// List registered tools
    Tools,
    /// List all sessions
    List,
    /// Clear all memory
    Clear {
        /// Confirm clearing
        #[arg(long)]
        yes: bool,
    },
    /// Show lab status and stats
    Status,
}

#[derive(Subcommand)]
enum PipelineCommands {
    /// Run a full research pipeline
    Run {
        /// File pattern to analyze (glob)
        #[arg(long, default_value = "**/*.py")]
        pattern: String,
        /// Path to filter by (optional)
        #[arg(long)]
        path: Option<String>,
        /// Disable the review phase
        #[arg(long, default_value = "false")]
        no_review: bool,
        /// Disable code generation
        #[arg(long, default_value = "false")]
        no_code: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { workspace } => {
            let ws = if workspace != "." {
                let p = PathBuf::from(&workspace);
                std::fs::create_dir_all(&p)?;
                p
            } else {
                std::env::current_dir()?
            };
            let config = LabConfig::with_workspace(ws.clone());
            println!(
                "{}",
                format!("Initialized AI Research Lab workspace: {}", ws.display())
                    .green()
                    .bold()
            );
            println!();
            println!(
                "Workspace dirs:  {}  Memory: {}  Outputs: {}",
                config.full_path(&config.sessions_dir).display(),
                config.full_path(&config.memory_dir).display(),
                config.full_path(&config.outputs_dir).display(),
            );
            println!();
            println!("Run `lab pipeline run` to start a full research task.");
        }

        Commands::Pipeline { subcommand } => match subcommand {
            PipelineCommands::Run { pattern, path, no_review, no_code } => {
                println!("{} Starting Pipeline: ...", "▶".cyan().bold());
                println!();

                let config = LabConfig::default();
                let workspace = config.workspace.clone();
                let memory_dir = config.full_path(&config.memory_dir);
                let mut lab = ResearchLab::new(config);
                lab.start().await?;

                let session = lab.create_session("pipeline-run").await?;
                let session_id = session.id.clone();

                // Create fresh components for pipeline
                let mut registry = lab_tools::ToolRegistry::new(workspace);
                registry.register_builtins();
                let mut mem = lab_memory::MemoryWorkspace::new(memory_dir);

                let mut pipeline = lab_pipelines::ResearchPipeline::new(
                    lab_pipelines::PipelineConfig::default()
                        .with_name("cli-pipeline")
                        .with_stages(vec![
                            "discover".into(),
                            "research".into(),
                            "analyze".into(),
                            "summarize".into(),
                        ]),
                    session_id.clone(),
                );

                let result = pipeline.run(&mut registry, &mut mem).await;

                println!();
                println!("{}", "═══ Pipeline Results ═══".bold().cyan());
                println!("Status: {}", match result.status.as_str() {
                    "completed" => "✅ Completed".green(),
                    "failed" => "❌ Failed".red(),
                    _ => "⚠️ Partial".yellow(),
                });
                for stage in &result.stage_results {
                    let icon = if matches!(stage.status, lab_pipelines::StageStatus::Completed) { "✅" } else { "❌" };
                    println!("  {} {} ({:.1}s)", icon, stage.name.bold(), stage.duration_secs);
                    if let Some(e) = &stage.error {
                        println!("    {}", e.red().dimmed());
                    }
                }
                println!();
                println!("Total Duration: {:.1}s", result.total_duration_secs);

                lab.close_session(&session_id).await?;
                lab.shutdown().await?;
            }
        },


        Commands::Ask { question } => {
            println!("{} {}", "🤔 Asking LLM:".cyan().bold(), question);
            println!();
            
            let config = LabConfig::default();
            let workspace = config.workspace.clone();
            let lab = ResearchLab::new(config);
            
            if !lab.has_llm() {
                eprintln!("{} LLM is not configured. Set API key via environment variables.", "❌".red());
                eprintln!("   Expected: export ANTHROPIC_API_KEY='...'");
                std::process::exit(1);
            }

            // Auto-gather context: Read key files to give LLM actual code
            let mut context = format!("Project root: {}\n\n", workspace.display());
            
            let key_files = vec!["Cargo.toml", "README.md", "DESIGN.md"];
            for file_name in key_files {
                let file_path = workspace.join(file_name);
                if file_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&file_path) {
                        let preview = if content.len() > 3000 {
                            format!("{}\n...[truncated]", &content[..3000])
                        } else {
                            content.clone()
                        };
                        context.push_str(&format!("=== {file_name} ===\n{preview}\n\n"));
                    }
                }
            }
            
            // List top directories
            if let Ok(entries) = std::fs::read_dir(&workspace) {
                let mut dirs = Vec::new();
                for entry in entries.filter_map(|e| e.ok()) {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        dirs.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
                context.push_str(&format!("Top-level directories: {}\n", dirs.join(", ")));
            }
            
            // Read main binary entry point
            let main_rs = workspace.join("crates/lab-cli/src/main.rs");
            if main_rs.exists() {
                if let Ok(content) = std::fs::read_to_string(&main_rs) {
                    let preview = if content.len() > 2000 { &content[..2000] } else { &content };
                    context.push_str(&format!("\n=== crates/lab-cli/src/main.rs (first 2000 chars) ===\n{preview}\n"));
                }
            }

            let system_prompt = "You are an expert AI code reviewer. Analyze the provided project files and answer the user's question clearly. Be concise.";
            let full_prompt = format!("{context}\n\nUser Question: {question}");

            let response = lab.ask_llm(&full_prompt, system_prompt, 0.2, 4096).await;
            
            if let Some(text) = response {
                println!("{text}");
            } else {
                eprintln!("{} No response received from LLM.", "❌".red());
            }
        }

        Commands::Serve { port } => {
            println!("{} Binding to 0.0.0.0:{}", "🚀 Starting API Server:".green().bold(), port);
            let config = LabConfig::default();
            config.ensure_directories();
            
            let state = Arc::new(lab_api::AppState::new(config).await);
            let router = lab_api::create_router(state.clone());
            let addr = format!("0.0.0.0:{}", port);
            
            println!("🌐 REST Endpoints:");
            println!("  GET  /sessions");
            println!("  POST /sessions");
            println!("  GET  /tools");
            println!("  POST /tools/execute");
            println!("  GET  /memory/:id");
            println!("  POST /pipelines/run");
            println!("  WS   /events (Real-time logs)");
            println!();

            let listener = tokio::net::TcpListener::bind(&addr).await?;
            axum::serve(listener, router).await?;
        }

        Commands::Tools => {
            let config = LabConfig::default();
            let lab = ResearchLab::new(config);
            let tools = lab.tools().list_tools();
            println!(
                "{} {}",
                "═ Registered tools:".bold().yellow(),
                format!("({} total) =", tools.len()).bold().yellow()
            );
            for tool in &tools {
                let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let desc = tool.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let category = tool.get("category").and_then(|v| v.as_str()).unwrap_or("");
                println!("  {} — {} [{}]", name.bold(), desc, category);
            }
        }

        Commands::List => {
            let config = LabConfig::default();
            let mut lab = ResearchLab::new(config);
            lab.start().await?;
            let _ = lab.create_session("temp").await;
            
            let sessions = lab.list_sessions();
            println!(
                "{} {}",
                "═ Sessions:".bold().cyan(),
                format!("({})", sessions.len()).bold().cyan()
            );
            for session in &sessions {
                println!(
                    "  • [{}] {} — {:?} ({} agents)",
                    session.id, session.name, session.status, session.agents_active
                );
            }
            lab.shutdown().await?;
        }

        Commands::Clear { yes } => {
            let config = LabConfig::default();
            let lab = ResearchLab::new(config);
            let count = lab.memory().entry_count();
            if count == 0 {
                println!("{} Memory is already empty.", "ℹ".yellow());
            } else if yes {
                let ws = &lab.memory().workspace;
                if ws.exists() {
                    std::fs::remove_dir_all(ws)?;
                }
                println!(
                    "{} Cleared {} entries at {}",
                    "✓".green().bold(),
                    count,
                    ws.display()
                );
            } else {
                println!(
                    "{} Would clear {} entries. Use `lab clear --yes` to confirm.",
                    "⚠".yellow().bold(),
                    count
                );
            }
        }

        Commands::Status => {
            let config = LabConfig::default();
            println!("{}", "═══ AI Research Lab Status ═══".bold().cyan());
            
            // Check Workspace
            let ws = &config.workspace;
            if ws.exists() {
                println!("{} {}", "Workspace:".dimmed(), ws.display());
            } else {
                println!("{} No workspace found. Run `lab init` first.", "⚠️".yellow());
            }

            // Check Sessions
            let sessions_dir = config.full_path(&config.sessions_dir);
            let session_count = if sessions_dir.exists() {
                std::fs::read_dir(&sessions_dir)
                    .map(|d| d.filter(|e| e.as_ref().map(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)).unwrap_or(false)).count())
                    .unwrap_or(0)
            } else {
                0
            };
            println!("{} {} (at {})", "Sessions:".dimmed(), session_count, sessions_dir.display());

            // Check Memory
            let memory_dir = config.full_path(&config.memory_dir);
            let memory_entries = if memory_dir.exists() {
                std::fs::read_dir(&memory_dir)
                    .map(|d| d.filter(|e| e.as_ref().map(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).unwrap_or(false)).count())
                    .unwrap_or(0)
            } else {
                0
            };
            println!("{} {} (at {})", "Memory:".dimmed(), memory_entries, memory_dir.display());

            // Check Outputs
            let outputs_dir = config.full_path(&config.outputs_dir);
            let outputs_count = if outputs_dir.exists() {
                std::fs::read_dir(&outputs_dir)
                    .map(|d| d.filter(|e| e.as_ref().map(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)).unwrap_or(false)).count())
                    .unwrap_or(0)
            } else {
                0
            };
            println!("{} {}", "Outputs:".dimmed(), outputs_count);
        }
    }

    Ok(())
}
