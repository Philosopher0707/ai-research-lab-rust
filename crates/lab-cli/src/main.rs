mod commands;
mod setup;
mod ui;

use clap::{Parser, Subcommand};
use colored::Colorize;
use lab_core::LabConfig;

#[derive(Parser)]
#[command(
    name = "lab",
    about = "AI Research Lab — interactive agentic CLI",
    version
)]
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
        #[arg(long, default_value = "**/*.rs")]
        pattern: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long, default_value = "false")]
        no_review: bool,
        #[arg(long, default_value = "false")]
        no_code: bool,
    },
    /// Ask a question about your codebase (single-shot)
    Ask { question: String },
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
    /// Run a specific agent on the workspace
    Agent {
        /// Agent type: researcher | reviewer | coder | summarizer
        #[arg(long, default_value = "researcher")]
        r#type: String,
        /// Glob pattern for files to process
        #[arg(long, default_value = "**/*.rs")]
        pattern: String,
    },
    /// Search the web using DuckDuckGo
    Search {
        query: String,
        /// Maximum number of results to return
        #[arg(long, default_value_t = 5)]
        max_results: u8,
    },
    /// Inspect the lab memory store
    Memory {
        /// Show entries for a specific session ID
        #[arg(long)]
        session: Option<String>,
        /// Print the value of a specific key
        #[arg(long)]
        key: Option<String>,
    },
    /// Execute a named tool with JSON params
    Tool {
        /// Tool name (e.g. read_file, web_search)
        name: String,
        /// JSON object of parameters, e.g. '{"path":"src/main.rs"}'
        #[arg(long, default_value = "{}")]
        params: String,
    },
    /// Interactive wizard: pick provider, model, and enter API key
    Setup,
    /// Full autonomous self-improvement: research → plan → implement → verify
    Improve {
        /// Glob pattern for files to analyse (default: **/*.rs)
        #[arg(long, default_value = "**/*.rs")]
        pattern: String,
        /// Maximum improvement candidates to generate
        #[arg(long, default_value_t = 10)]
        max_candidates: usize,
        /// Maximum improvements to apply per run
        #[arg(long, default_value_t = 5)]
        max_apply: usize,
        /// Plan only — show candidates without applying
        #[arg(long)]
        dry_run: bool,
        /// Also run cargo test after each edit (slower)
        #[arg(long)]
        run_tests: bool,
        /// Skip creating a git branch
        #[arg(long)]
        no_branch: bool,
        /// Search the web for context before planning
        #[arg(long)]
        web_research: bool,
    },
    /// LLM-driven self-improvement: edit a file and verify with cargo check
    SelfEdit {
        /// File to improve (relative to workspace, e.g. crates/lab-tools/src/lib.rs)
        #[arg(long)]
        file: String,
        /// Natural-language description of what to improve
        #[arg(long)]
        task: String,
        /// Show proposed edits without applying them
        #[arg(long)]
        dry_run: bool,
    },
    /// Open or view the latest HTML research report
    Report {
        /// Open report in the default browser
        #[arg(long)]
        open: bool,
        /// Path to write/read the HTML report (default: lab-outputs/pipeline-report.html)
        #[arg(long)]
        output: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    LabConfig::initialize_process_env();

    // Initialise tracing — info level by default, debug if LAB_DEBUG=1
    let log_level = if std::env::var("LAB_DEBUG").ok().as_deref() == Some("1") {
        tracing::Level::DEBUG
    } else {
        tracing::Level::WARN // keep CLI output clean
    };
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    if let Err(e) = dispatch(cli).await {
        let msg = format!("{e:#}");
        if !msg.is_empty() {
            eprintln!("{} {}", "●".red().bold(), msg);
        }
        std::process::exit(1);
    }
}

async fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        None => commands::cmd_chat().await,
        Some(Commands::Init { workspace }) => commands::cmd_init(&workspace),
        Some(Commands::Pipeline {
            pattern,
            path,
            no_review,
            no_code,
        }) => commands::cmd_pipeline_run(&pattern, path.as_deref(), no_review, no_code).await,
        Some(Commands::Ask { question }) => commands::cmd_ask_once(&question).await,
        Some(Commands::Serve { port }) => commands::cmd_serve(port).await,
        Some(Commands::Tools) => commands::cmd_tools().await,
        Some(Commands::List) => commands::cmd_list().await,
        Some(Commands::Clear { yes }) => commands::cmd_clear(yes),
        Some(Commands::Status) => commands::cmd_status().await,
        Some(Commands::Agent { r#type, pattern }) => commands::cmd_agent(&r#type, &pattern).await,
        Some(Commands::Search { query, max_results }) => {
            commands::cmd_search(&query, max_results).await
        }
        Some(Commands::Memory { session, key }) => {
            commands::cmd_memory(session.as_deref(), key.as_deref()).await
        }
        Some(Commands::Tool { name, params }) => commands::cmd_tool(&name, &params).await,
        Some(Commands::Setup) => setup::cmd_setup().await,
        Some(Commands::Improve {
            pattern,
            max_candidates,
            max_apply,
            dry_run,
            run_tests,
            no_branch,
            web_research,
        }) => {
            commands::cmd_improve(
                &pattern,
                max_candidates,
                max_apply,
                dry_run,
                run_tests,
                !no_branch,
                web_research,
            )
            .await
        }
        Some(Commands::SelfEdit {
            file,
            task,
            dry_run,
        }) => commands::cmd_self_edit(&file, &task, dry_run).await,
        Some(Commands::Report { open, output }) => {
            commands::cmd_report(open, output.as_deref()).await
        }
    }
}
