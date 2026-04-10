use colored::Colorize;
use lab_core::LabConfig;
use std::future::Future;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn provider_display_name(provider: &str) -> &str {
    match provider {
        "anthropic" => "Anthropic",
        "openai" => "OpenAI",
        "openrouter" => "OpenRouter",
        "deepseek" => "DeepSeek",
        "zhipu" => "ZhipuAI",
        "minimax" => "MiniMax",
        "xai" => "xAI",
        "local" => "Local",
        _ => provider,
    }
}

pub(crate) fn print_rule() {
    println!("{}", "─".repeat(72).bright_black());
}

fn print_ascii_banner() {
    for line in [
        "    __    ___    ____ ",
        "   / /   /   |  / __ )",
        "  / /   / /| | / __  |",
        " / /___/ ___ |/ /_/ / ",
        "/_____/_/  |_/_____/  ",
    ] {
        println!("{}", line.bright_red().bold());
    }
    println!("{}", "AI Research Lab".bright_red().bold());
    println!("{}", "═".repeat(72).bright_red());
}

pub(crate) fn print_meta_row(label: &str, value: impl std::fmt::Display) {
    println!("  {} {}", format!("{label:<10}").bright_black(), value);
}

pub(crate) fn llm_setup_hint(config: &LabConfig) -> String {
    if config.provider == "local" {
        return "run `lab setup` or add LAB_PROVIDER=local and LAB_BASE_URL to .env".into();
    }

    match config.expected_api_key_env() {
        Some(env_key) => format!("run `lab setup` or add {env_key} to .env"),
        None => "run `lab setup` to configure a provider".into(),
    }
}

pub(crate) fn print_help_menu() {
    println!();
    println!("{}", "Commands".bold().bright_white());
    print_rule();
    println!(
        "  {} {}",
        "/help".cyan().bold(),
        "Show available commands".dimmed()
    );
    println!(
        "  {} {}",
        "/status".cyan().bold(),
        "Show workspace and provider status".dimmed()
    );
    println!(
        "  {} {}",
        "/pipeline".cyan().bold(),
        "Run the default research pipeline".dimmed()
    );
    println!(
        "  {} {}",
        "/clear".cyan().bold(),
        "Clear only the current chat history".dimmed()
    );
    println!(
        "  {} {}",
        "/exit".cyan().bold(),
        "Leave the interactive session".dimmed()
    );
    println!(
        "  {} {}",
        "[question]".cyan().bold(),
        "Ask about architecture, Rust, debugging, or workflows".dimmed()
    );
    println!();
}

pub(crate) fn print_chat_header(config: &LabConfig, llm_ready: bool) {
    println!();
    print_ascii_banner();
    println!(
        "{}",
        "Interactive multi-agent research workspace for codebases.".dimmed()
    );
    println!("{}", "· developer shell ·".bright_red().bold());
    print_rule();
    print_meta_row("Workspace", config.workspace.display());
    print_meta_row(
        "Provider",
        provider_display_name(&config.provider).cyan().bold(),
    );
    print_meta_row("Model", config.model.as_str());
    print_meta_row(
        "LLM",
        if llm_ready {
            "Ready".green().bold().to_string()
        } else {
            "Needs setup".yellow().bold().to_string()
        },
    );
    if llm_ready && config.provider == "local" {
        print_meta_row("Endpoint", config.base_url.as_str().dimmed());
    } else if !llm_ready {
        print_meta_row("Hint", llm_setup_hint(config).yellow());
    }
    print_meta_row(
        "Config",
        ".env is loaded automatically from the workspace".dimmed(),
    );
    print_rule();
    println!(
        "  {} {}",
        "Slash".bright_black(),
        "/help  /status  /pipeline  /clear  /exit".cyan()
    );
    println!(
        "  {} {}",
        "Prompt".bright_black(),
        "Ask a question and press Enter.".dimmed()
    );
    println!();
}

pub(crate) fn print_success(message: impl std::fmt::Display) {
    println!("{} {}", "●".green().bold(), message);
}

pub(crate) fn print_warning(message: impl std::fmt::Display) {
    eprintln!("{} {}", "●".yellow().bold(), message);
}

pub(crate) fn print_error(message: impl std::fmt::Display) {
    eprintln!("{} {}", "●".red().bold(), message);
}

pub(crate) async fn with_spinner<F, T>(label: &str, future: F) -> T
where
    F: Future<Output = T>,
{
    let done = Arc::new(AtomicBool::new(false));
    let done_signal = Arc::clone(&done);
    let label = label.to_string();

    let spinner = tokio::spawn(async move {
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let mut frame = 0usize;

        while !done_signal.load(Ordering::Relaxed) {
            print!(
                "\r\x1b[2K{} {}",
                frames[frame % frames.len()].cyan().bold(),
                label.dimmed()
            );
            let _ = std::io::stdout().flush();
            tokio::time::sleep(Duration::from_millis(90)).await;
            frame += 1;
        }
    });

    let output = future.await;
    done.store(true, Ordering::Relaxed);
    let _ = spinner.await;
    print!("\r\x1b[2K");
    let _ = std::io::stdout().flush();
    output
}
