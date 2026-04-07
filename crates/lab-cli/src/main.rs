use clap::{Parser, Subcommand};
use colored::Colorize;
use lab_api;
use lab_core::{LabConfig, ResearchLab};
use std::path::PathBuf;
use std::sync::Arc;

fn banner() {
    // L      A       B
    // L     A A      B B
    // L    A   A     B  B
    // L    AAAAA     B BBB
    // L    A   A     B   B
    // LLLL A   A     BBBBB

    for line in vec![
        format!("{}   {}  {}",
            "L".cyan().bold(), "A".cyan().bold(), "B".cyan().bold()),
        format!("{}  {} {}      {} {}",
            "L".cyan().bold(), "A".cyan(), "A".cyan(), "B".cyan(), "B".cyan()),
        format!("{} {}   {}     {}  {}",
            "L".cyan().bold(), "A".cyan(), "A".cyan(), "B".cyan().bold(), "B".cyan()),
        format!("{} {} {} {}    {} {}",
            "L".cyan().bold(), "A".cyan(), "A".cyan().bold(), "A".cyan(), "B".cyan().bold(), "B".cyan()),
        format!("{} {}   {}     {}  {}",
            "L".cyan().bold(), "A".cyan(), "A".cyan(), "B".cyan().bold(), "   B".cyan()),
        format!("{} {}   {}     {} {} {}",
            "L".cyan().bold(), "A".cyan(), "   A".cyan(), "B".cyan().bold(), "B".cyan(), "B".cyan()),
    ] {
        println!("{}", line);
    }
    println!("{} Code {}", "════".cyan().bold(), "🧪".cyan());
}

#[derive(Parser)]
#[command(name = "lab", about = "AI Research Lab — interactive agentic CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the lab workspace
    Init {
        #[arg(short, long, default_value = ".")]
        workspace: String,
    },
    /// Run a full research pipeline
    Pipeline {
        #[arg(long, default_value = "**/*.py")]
        pattern: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long, default_value = "false")]
        no_review: bool,
        #[arg(long, default_value = "false")]
        no_code: bool,
    },
    /// Ask a question about your codebase using LLM
    Ask {
        question: String,
    },
    /// Serve the REST API + WebSocket server
    Serve {
        #[arg(short, long, default_value = "8000")]
        port: u16,
    },
    /// List registered tools
    Tools,
    /// List all sessions
    List,
    /// Clear all memory
    Clear {
        #[arg(long)]
        yes: bool,
    },
    /// Show lab status and stats
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Check if any CLI flag like --help or --version is present
    let args: Vec<String> = std::env::args().collect();
    let wants_help = args.iter().any(|a| a == "--help" || a == "-h");
    let wants_version = args.iter().any(|a| a == "--version" || a == "-V");

    if wants_help {
        Cli::parse(); // clap will print help and exit
        return Ok(());
    }
    if wants_version {
        Cli::parse(); // clap will print version and exit
        return Ok(());
    }

    let cli = Cli::parse();

    if cli.command.is_none() {
        return cmd_chat().await;
    }

    match cli.command.unwrap() {
        Commands::Init { workspace } => cmd_init(&workspace),
        Commands::Pipeline { pattern, path, no_review, no_code } => {
            cmd_pipeline_run(&pattern, path.as_deref(), no_review, no_code).await
        }
        Commands::Ask { question } => cmd_ask(&question).await,
        Commands::Serve { port } => cmd_serve(port).await,
        Commands::Tools => cmd_tools(),
        Commands::List => cmd_list().await,
        Commands::Clear { yes } => cmd_clear(yes),
        Commands::Status => cmd_status().await,
    }
}

async fn cmd_chat() -> anyhow::Result<()> {
    banner();
    println!();

    let config = LabConfig::default();
    let lab = ResearchLab::new(config.clone());

    let status_str: String = if lab.has_llm() {
        format!("● Connected ({})", config.model).green().to_string()
    } else {
        "○ Not configured".yellow().to_string()
    };

    println!("{} {}", "  Model:".dimmed(), config.model.dimmed());
    println!("{} {}", "  LLM:".dimmed(), status_str);
    println!();
    println!("{}", "  Just type your question — no prefix needed.".dimmed());
    println!("{} /help    /status    /pipeline    /exit", "  ".dimmed());
    println!("{}", "═══════════════════════════════════════════".dimmed());
    println!();

    loop {
        print!("❯ ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).unwrap_or(0) == 0 { break; }
        let input = input.trim().to_string();
        if input.is_empty() { continue; }

        if input == "/exit" || input == "/quit" || input == "/q" {
            println!(); println!("{}", "Goodbye 👋".dimmed()); break;
        }
        if input == "/help" || input == "/?" {
            println!();
            println!("  /help        ─ Show available commands");
            println!("  /status      ─ Show lab workspace status");
            println!("  /pipeline    ─ Run code scan on current dir");
            println!("  /exit        ─ Quit");
            println!("  [message]    ─ Ask AI about your codebase");
            println!();
            continue;
        }
        if input == "/status" {
            let _ = cmd_status().await;
            println!();
            continue;
        }
        if input == "/pipeline" {
            let _ = cmd_pipeline_run("**/*.*", None, false, false).await;
            println!();
            continue;
        }

        // Direct chat — send to LLM
        if lab.has_llm() {
            if let Err(e) = cmd_ask(&input).await { eprintln!("{} {}", "❌".red(), e); }
        } else {
            eprintln!("{} LLM not configured. Set API key in .zshrc.", "⚠".yellow());
        }
        println!();
    }
    Ok(())
}

fn cmd_init(workspace: &str) -> anyhow::Result<()> {
    let ws = if workspace != "." {
        let p = PathBuf::from(workspace); std::fs::create_dir_all(&p)?; p
    } else { std::env::current_dir()? };
    let config = LabConfig::with_workspace(ws.clone());
    println!("{}", format!("Initialized workspace: {}", ws.display()).green().bold());
    println!("\nWorkspace:   {}\nMemory:      {}\nOutputs:     {}",
        config.full_path(&config.sessions_dir).display(),
        config.full_path(&config.memory_dir).display(),
        config.full_path(&config.outputs_dir).display());
    println!("\nRun `lab` to start chat mode.");
    Ok(())
}

async fn cmd_pipeline_run(pattern: &str, _path: Option<&str>, no_review: bool, _no_code: bool) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let ws = config.workspace.clone();
    let md = config.full_path(&config.memory_dir);
    println!("🔬 {} Pipeline...", if no_review { "Quick" } else { "Full" });
    let mut stages = vec!["discover".into(), "research".into(), "analyze".into()];
    if !no_review { stages.push("review".into()); }
    stages.push("summarize".into());
    let pc = lab_pipelines::PipelineConfig {
        name: "cli-pipeline".into(), stages, input_targets: vec![pattern.into()],
        max_concurrent_stages: 1, fail_fast: true, retry_on_failure: false,
        timeout_per_stage_secs: 1800, output_path: None, exclude_patterns: vec![],
    };
    let mut pipeline = lab_pipelines::ResearchPipeline::new(pc, "pipeline".to_string());
    let mut registry = lab_tools::ToolRegistry::new(ws);
    registry.register_builtins();
    let mut mem = lab_memory::MemoryWorkspace::new(md);
    let result = pipeline.run(&mut registry, &mut mem, None, None).await;
    println!("\n{}", "═══ Pipeline Results ═══".bold().cyan());
    println!("Status: {}", match result.status.as_str() {
        "completed" => "✅ Completed".green(), "failed" => "❌ Failed".red(), _ => "⚠️ Partial".yellow(),
    });
    for s in &result.stage_results {
        let i = if matches!(s.status, lab_pipelines::StageStatus::Completed) { "✅" } else { "❌" };
        println!("  {} {} ({:.1}s)", i, s.name.bold(), s.duration_secs);
    }
    println!("\nTotal: {:.1}s", result.total_duration_secs);
    Ok(())
}

async fn cmd_ask(question: &str) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let ws = config.workspace.clone();
    let lab = ResearchLab::new(config);
    if !lab.has_llm() { eprintln!("{} LLM not configured.", "❌".red()); return Ok(()); }
    let mut ctx = format!("Project: {}\n\n", ws.display());
    for f in &["Cargo.toml", "README.md", "DESIGN.md"] {
        if let Ok(c) = std::fs::read_to_string(ws.join(f)) {
            let p = if c.len() > 2000 { &c[..2000] } else { &c };
            ctx.push_str(&format!("=== {} ===\n{}\n\n", f, p));
        }
    }
    if let Some(text) = lab.ask_llm(&format!("{}\n\nQ: {}", ctx, question), "Be concise and direct.", 0.2, 1024).await {
        println!("{}", text);
    } else {
        eprintln!("{} No LLM response.", "❌".red());
    }
    Ok(())
}

async fn cmd_serve(port: u16) -> anyhow::Result<()> {
    let config = LabConfig::default();
    config.ensure_directories();
    let state = Arc::new(lab_api::AppState::new(config).await);
    let router = lab_api::create_router(state.clone());
    let addr = format!("0.0.0.0:{}", port);
    println!("🌐 Server on {}", addr.bold().cyan());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

fn cmd_tools() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config);
    let tools = lab.tools().list_tools();
    println!("{}", format!("═══ {} tools:", tools.len()).bold().yellow());
    for t in &tools {
        let n = t.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let d = t.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let c = t.get("category").and_then(|v| v.as_str()).unwrap_or("");
        println!("  {} — {} [{}]", n.bold(), d, c);
    }
    Ok(())
}

async fn cmd_list() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let sd = config.full_path(&config.sessions_dir);
    let count = if sd.exists() { std::fs::read_dir(&sd).map(|d| d.count()).unwrap_or(0) } else { 0 };
    println!("{}", format!("═══ {} session(s)", count).bold().cyan());
    Ok(())
}

fn cmd_clear(yes: bool) -> anyhow::Result<()> {
    if yes {
        let config = LabConfig::default();
        let md = config.full_path(&config.memory_dir);
        if md.exists() { std::fs::remove_dir_all(&md)?; }
        std::fs::create_dir_all(&md)?;
        println!("{} Cleared.", "✓".green().bold());
    } else {
        println!("{} Use `lab clear --yes`.", "⚠".yellow().bold());
    }
    Ok(())
}

async fn cmd_status() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config.clone());
    println!("{}", "═══ AI Research Lab ═══".bold().cyan());
    println!("{} {}", "Workspace:".dimmed(), lab.config.workspace.display());
    println!("{} {} ({})", "Provider:".dimmed(), lab.config.provider, lab.config.model);
    println!("{} {}", "Tools:".dimmed(), lab.tools().list_tools().len());
    let sd = config.full_path(&config.sessions_dir);
    let sc = if sd.exists() { std::fs::read_dir(&sd).map(|d| d.count()).unwrap_or(0) } else { 0 };
    println!("{} {}", "Sessions:".dimmed(), sc);
    println!("{} {}", "Memory:".dimmed(), lab.memory().entry_count());
    println!("{} {}", "LLM:".dimmed(), if lab.has_llm() {
        format!("✅ ({})", lab.config.model)
    } else { "○ Not set".to_string() });
    Ok(())
}
