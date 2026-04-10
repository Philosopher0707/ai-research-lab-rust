mod commands;
mod setup;
mod ui;

use clap::{Parser, Subcommand};
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
    /// Interactive wizard: pick provider, model, and enter API key
    Setup,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
        Some(Commands::Tools) => commands::cmd_tools(),
        Some(Commands::List) => commands::cmd_list().await,
        Some(Commands::Clear { yes }) => commands::cmd_clear(yes),
        Some(Commands::Status) => commands::cmd_status().await,
        Some(Commands::Agent { r#type, pattern }) => commands::cmd_agent(&r#type, &pattern).await,
        Some(Commands::Setup) => setup::cmd_setup().await,
    }
}
