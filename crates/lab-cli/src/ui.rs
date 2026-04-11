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

/// Return a colored category label for tool listings.
pub(crate) fn category_color(category: &str) -> colored::ColoredString {
    match category {
        "filesystem" => format!("  [{category}]").yellow().bold(),
        "network" => format!("  [{category}]").cyan().bold(),
        "system" => format!("  [{category}]").red().bold(),
        "vcs" => format!("  [{category}]").bright_blue().bold(),
        _ => format!("  [{category}]").bright_black().bold(),
    }
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
    println!("{}", "Chat commands".bold().bright_white());
    print_rule();

    let cmds: &[(&str, &str)] = &[
        ("/help",          "Show this menu"),
        ("/status",        "Workspace and provider status"),
        ("/tools",         "List all registered tools by category"),
        ("/memory",        "Inspect the memory store"),
        ("/search <q>",    "Web search via DuckDuckGo"),
        ("/fetch <url>",   "Fetch a URL and print its content"),
        ("/pipeline",      "Run the default research pipeline"),
        ("/report",        "Open the latest HTML report in browser"),
        ("/clear",         "Clear current chat history"),
        ("/exit",          "Leave the session"),
        ("[question]",     "Ask anything about the project or Rust"),
    ];

    for (cmd, desc) in cmds {
        println!(
            "  {}  {}",
            format!("{cmd:<22}").cyan().bold(),
            desc.dimmed()
        );
    }
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
        "/help  /tools  /search <q>  /fetch <url>  /memory  /pipeline  /report  /clear  /exit".cyan()
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
    // Skip spinner when not connected to a terminal or when disabled explicitly
    let is_tty = atty::is(atty::Stream::Stdout);
    let disabled = std::env::var("NO_SPINNER").is_ok();

    if !is_tty || disabled {
        return future.await;
    }

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
