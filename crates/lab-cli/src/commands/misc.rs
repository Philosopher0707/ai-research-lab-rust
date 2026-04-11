use crate::ui::{
    llm_setup_hint, print_meta_row, print_rule, print_success, print_warning,
    provider_display_name, with_spinner,
};
use colored::Colorize;
use lab_core::llm::ChatMessage;
use lab_core::{LabConfig, ResearchLab};
use std::path::{Path, PathBuf};

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

pub(crate) async fn cmd_ask_once(question: &str) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config.clone());
    if !lab.has_llm() {
        anyhow::bail!("LLM unavailable — {}.", llm_setup_hint(&config));
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
        None => anyhow::bail!("No response from the configured LLM."),
    }
    Ok(())
}

pub(crate) async fn cmd_tools() -> anyhow::Result<()> {
    use crate::ui::category_color;
    use std::collections::BTreeMap;

    let config = LabConfig::default();
    let lab = ResearchLab::new(config);
    let tools = lab.tools().list_tools().await;

    println!();
    println!(
        "{}",
        format!("Tools  ({} registered)", tools.len())
            .bright_white()
            .bold()
    );
    print_rule();

    let mut by_cat: BTreeMap<String, Vec<&serde_json::Value>> = BTreeMap::new();
    for tool in &tools {
        let cat = tool
            .get("category")
            .and_then(|v: &serde_json::Value| v.as_str())
            .unwrap_or("custom")
            .to_string();
        by_cat.entry(cat).or_default().push(tool);
    }

    for (category, group) in &by_cat {
        println!("  {}", category_color(category));
        for tool in group {
            let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let desc = tool
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            println!("    {}  {}", format!("{name:<20}").bold(), desc.dimmed());
        }
        println!();
    }
    Ok(())
}

pub(crate) async fn cmd_list() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let sessions_dir = config.full_path(&config.sessions_dir);

    let index_path = sessions_dir.join("index.json");
    let mut records: Vec<(String, serde_json::Value)> = Vec::new();

    if let Ok(content) = std::fs::read_to_string(&index_path) {
        if let Ok(map) =
            serde_json::from_str::<std::collections::HashMap<String, serde_json::Value>>(&content)
        {
            records = map.into_iter().collect();
        }
    }

    records.sort_by(|a, b| {
        let ta = a.1.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        let tb = b.1.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        tb.cmp(ta)
    });

    println!();
    println!("{}", "Sessions".bright_white().bold());
    print_rule();

    if records.is_empty() {
        println!("{}", "  No sessions found.".dimmed());
        return Ok(());
    }

    for (id, rec) in &records {
        let name = rec.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let status = rec.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let created = rec
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .get(..19)
            .unwrap_or("");
        let tasks = rec
            .get("tasks_completed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let status_str = match status {
            "completed" => status.green().to_string(),
            "failed" => status.red().to_string(),
            "active" => status.yellow().to_string(),
            _ => status.dimmed().to_string(),
        };

        println!(
            "  {} {} {}  {} tasks  {}",
            id.cyan().bold(),
            format!("{name:<28}").normal(),
            status_str,
            tasks.to_string().bright_black(),
            created.dimmed(),
        );
    }

    println!();
    print_meta_row("Total", records.len());
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
    print_meta_row("Tools", lab.tools().list_tools().await.len());
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

pub(crate) async fn cmd_search(query: &str, max_results: u8) -> anyhow::Result<()> {
    use lab_tools::Tool;

    println!();
    println!(
        "{}",
        format!("Web Search  ·  {query}").bright_white().bold()
    );
    print_rule();

    let tool = lab_tools::DuckDuckGoTool;
    let mut params = std::collections::HashMap::new();
    params.insert("query".to_string(), serde_json::json!(query));
    params.insert("max_results".to_string(), serde_json::json!(max_results));

    let result = with_spinner("Searching", tool.execute(&params)).await;

    if !result.success {
        anyhow::bail!(result.error.unwrap_or_else(|| "Search failed".to_string()));
    }

    let data = result.data.unwrap_or_default();

    if let Some(answer) = data
        .get("answer")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        println!("{} {}", "Answer".bright_yellow().bold(), answer);
        println!();
    }

    let results = data
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if results.is_empty() {
        println!("{}", "No results found.".dimmed());
    } else {
        for (i, item) in results.iter().enumerate() {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("(no title)");
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");

            println!("{}. {}", (i + 1).to_string().cyan().bold(), title.bold());
            if !url.is_empty() {
                println!("   {}", url.dimmed());
            }
            if !snippet.is_empty() && snippet != title {
                let display = if snippet.len() > 120 {
                    let end = snippet.floor_char_boundary(120);
                    format!("{}…", &snippet[..end])
                } else {
                    snippet.to_string()
                };
                println!("   {}", display.normal());
            }
            println!();
        }
    }

    Ok(())
}

pub(crate) async fn cmd_memory(
    session_filter: Option<&str>,
    key_filter: Option<&str>,
) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config.clone());
    let memory = lab.memory();
    let total = memory.entry_count();

    println!();
    println!("{}", "Memory Store".bright_white().bold());
    print_rule();
    print_meta_row("Total entries", total);
    print_meta_row("Backend", config.full_path(&config.memory_dir).display());
    print_rule();

    if total == 0 {
        println!("{}", "  (empty)".dimmed());
        return Ok(());
    }

    let mut sessions = memory.sessions();
    sessions.sort();

    for session_id in &sessions {
        if let Some(filter) = session_filter {
            if session_id != filter {
                continue;
            }
        }

        let mut keys = memory.keys_for_session(session_id);
        keys.sort();

        println!(
            "  {} {}",
            "session".bright_black(),
            session_id.cyan().bold()
        );

        if let Some(key) = key_filter {
            if let Some(value) = memory.get(session_id, key) {
                let pretty = serde_json::to_string_pretty(&value).unwrap_or_default();
                for line in pretty.lines() {
                    println!("    {line}");
                }
            } else {
                println!("    {}", format!("key '{key}' not found").yellow());
            }
        } else {
            for key in &keys {
                println!("    {}", key.bold());
            }
        }
        println!();
    }

    Ok(())
}

pub(crate) async fn cmd_tool(name: &str, params_json: &str) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config);

    let params: std::collections::HashMap<String, serde_json::Value> =
        match serde_json::from_str(params_json) {
            Ok(p) => p,
            Err(e) => anyhow::bail!("Invalid JSON params: {e}"),
        };

    if !lab.tools().get(name).await {
        anyhow::bail!("Tool '{name}' not found. Run `lab tools` to list available tools.");
    }

    println!();
    println!("{}", format!("Tool  ·  {name}").bright_white().bold());
    print_rule();

    let result = with_spinner(
        &format!("Running {name}"),
        lab.execute_tool(name, params, None),
    )
    .await;

    if result["success"].as_bool().unwrap_or(false) {
        let pretty = serde_json::to_string_pretty(&result["data"]).unwrap_or_default();
        println!("{pretty}");
    } else {
        let err = result["error"].as_str().unwrap_or("unknown error");
        anyhow::bail!("{err}");
    }

    Ok(())
}

pub(crate) async fn cmd_report(open: bool, output: Option<&str>) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let outputs_dir = config.full_path(&config.outputs_dir);

    let html_path: PathBuf = output
        .map(PathBuf::from)
        .unwrap_or_else(|| outputs_dir.join("pipeline-report.html"));

    println!();
    println!("{}", "Research Report".bright_white().bold());
    println!(
        "{}",
        "Self-contained HTML report from the last pipeline run.".dimmed()
    );
    print_rule();

    if !html_path.exists() {
        print_warning(format!("No report found at {}", html_path.display()));
        println!();
        print_meta_row("Generate one with", "lab pipeline".cyan());
        print_meta_row("Then open with", "lab report --open".cyan());
        return Ok(());
    }

    let size_kb = std::fs::metadata(&html_path)
        .map(|m| m.len() / 1024)
        .unwrap_or(0);

    print_meta_row("Path", html_path.display());
    print_meta_row("Size", format!("{size_kb} KB"));

    if open {
        open_in_browser(&html_path);
        println!();
        print_success("Opened in browser.");
    } else {
        println!();
        println!(
            "  {} {}",
            "Open in browser:".bright_black(),
            "lab report --open".cyan().bold()
        );
        println!(
            "  {} {}",
            "Direct path:    ".bright_black(),
            html_path.display().to_string().dimmed()
        );
    }

    Ok(())
}

fn open_in_browser(path: &Path) {
    let path_str = path.to_string_lossy();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg(path_str.as_ref())
        .spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open")
        .arg(path_str.as_ref())
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", path_str.as_ref()])
        .spawn();
}

pub(crate) fn session_record_count(path: &Path) -> usize {
    if path.exists() {
        std::fs::read_dir(path)
            .map(|entries| entries.count())
            .unwrap_or(0)
    } else {
        0
    }
}

fn build_project_context(config: &LabConfig) -> String {
    let mut context = String::new();
    for file_name in ["Cargo.toml", "RUNTIME_MAP.md", "DESIGN.md", "README.md"] {
        let path = config.workspace.join(file_name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            let snippet = if content.len() > 1500 {
                let end = content.floor_char_boundary(1500);
                &content[..end]
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
