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
    let lab = ResearchLab::new(config.clone());
    with_spinner("Booting workspace", lab.start()).await?;
    print_chat_header(&config, lab.has_llm());

    let tool_list = lab.tools().list_tools().await;
    let system_prompt = build_agent_system_prompt(&tool_list, &config);
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

        // Slash commands
        if input == "/exit" || input == "/quit" || input == "/q" {
            println!();
            println!("{}", "Session closed.".dimmed());
            break;
        } else if input == "/help" || input == "/?" {
            print_help_menu();
            continue;
        } else if input == "/status" {
            let _ = cmd_status().await;
            println!();
            continue;
        } else if input == "/pipeline" {
            let _ = cmd_pipeline_run("**/*.rs", None, false, true).await;
            println!();
            continue;
        } else if input == "/clear" {
            history.truncate(1);
            print_success("Conversation cleared.");
            println!();
            continue;
        } else if input == "/tools" {
            let _ = cmd_tools().await;
            println!();
            continue;
        } else if input == "/report" {
            let _ = cmd_report(true, None).await;
            println!();
            continue;
        } else if input == "/memory" {
            let _ = cmd_memory(None, None).await;
            println!();
            continue;
        } else if let Some(query) = input.strip_prefix("/search ").map(str::trim) {
            if query.is_empty() {
                print_warning("Usage: /search <query>");
            } else {
                let _ = cmd_search(query, 5).await;
            }
            println!();
            continue;
        } else if let Some(key) = input.strip_prefix("/fetch ").map(str::trim) {
            if key.is_empty() {
                print_warning("Usage: /fetch <url>");
            } else {
                let _ = cmd_tool("fetch_url", &format!(r#"{{"url":"{key}"}}"#)).await;
            }
            println!();
            continue;
        } else if input.starts_with('/') && input != "/" {
            print_warning(format!(
                "Unknown command '{}' — type /help for a list.",
                input
            ));
            println!();
            continue;
        }

        if !lab.has_llm() {
            print_warning(format!("LLM unavailable — {}.", llm_setup_hint(&config)));
            println!();
            continue;
        }

        history.push(ChatMessage::user(&input));

        // ── Tool dispatch loop ────────────────────────────────────
        let mut tool_rounds = 0;
        const MAX_TOOL_ROUNDS: usize = 8;

        'agent: loop {
            let response =
                match with_spinner("Thinking", lab.ask_llm_messages(history.clone(), 0.7, 4096))
                    .await
                {
                    Some(r) => r,
                    None => {
                        print_error("No response from the configured LLM.");
                        history.pop();
                        println!();
                        break 'agent;
                    }
                };

            let tool_calls = extract_tool_calls(&response);

            if tool_calls.is_empty() || tool_rounds >= MAX_TOOL_ROUNDS {
                // Final answer — strip any stray tags and display
                let answer = strip_tool_calls(&response);
                if !answer.is_empty() {
                    println!();
                    println!("{}", "assistant".bright_black().bold());
                    println!("{}", answer.trim());
                    println!();
                }
                history.push(ChatMessage::assistant(&response));
                break 'agent;
            }

            // Has tool calls — record assistant message
            history.push(ChatMessage::assistant(&response));

            // Print any reasoning text before the first tool call
            let preamble = strip_tool_calls(&response);
            if !preamble.is_empty() {
                println!();
                println!("{}", preamble.trim().dimmed());
            }
            println!();

            // Execute each tool and collect results
            let mut result_parts: Vec<String> = Vec::new();
            for call in &tool_calls {
                let tool_name = call.get("tool").and_then(|v| v.as_str()).unwrap_or("?");
                let params: std::collections::HashMap<String, serde_json::Value> = call
                    .get("params")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();

                // Show what we're calling
                let params_json =
                    serde_json::to_string(&call.get("params").cloned().unwrap_or_default())
                        .unwrap_or_default();
                let preview = if params_json.len() > 55 {
                    format!("{}…", &params_json[..55])
                } else {
                    params_json.clone()
                };
                print!(
                    "  {} {}  {}",
                    "⚙".bright_black(),
                    tool_name.cyan().bold(),
                    preview.dimmed()
                );
                let _ = std::io::stdout().flush();

                let result = lab.execute_tool(tool_name, params, None).await;
                let success = result["success"].as_bool().unwrap_or(false);

                let summary = if success {
                    result
                        .get("data")
                        .map(|d| summarize_tool_result(tool_name, d))
                        .unwrap_or_else(|| "ok".into())
                } else {
                    result["error"].as_str().unwrap_or("error").to_string()
                };

                if success {
                    println!("  → {} {}", "✓".green(), summary.dimmed());
                } else {
                    println!("  → {} {}", "✗".red(), summary.yellow());
                }

                result_parts.push(format!(
                    "[TOOL: {tool_name}]\n{}",
                    serde_json::to_string_pretty(&result).unwrap_or_default()
                ));
            }
            println!();

            // Feed results back as a user message and loop
            history.push(ChatMessage::user(result_parts.join("\n\n")));
            tool_rounds += 1;
        }
        // ─────────────────────────────────────────────────────────

        // Trim history: keep system + last ~24 messages
        if history.len() > 26 {
            let system = history[0].clone();
            history.drain(1..5);
            history[0] = system;
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
        let lab = &state.lab;
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

    // Group by category, preserving alphabetical order within each group
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

    // Sessions are stored in index.json as {session_id: record_object}
    let index_path = sessions_dir.join("index.json");
    let mut records: Vec<(String, serde_json::Value)> = Vec::new();

    if let Ok(content) = std::fs::read_to_string(&index_path) {
        if let Ok(map) =
            serde_json::from_str::<std::collections::HashMap<String, serde_json::Value>>(&content)
        {
            records = map.into_iter().collect();
        }
    }

    // Sort by created_at descending (newest first)
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
        dry_run: false,
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
        print_error(result.error.unwrap_or_else(|| "Search failed".to_string()));
        return Ok(());
    }

    let data = result.data.unwrap_or_default();

    // Instant answer / abstract
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
                // Wrap long snippets at 72 chars
                let display = if snippet.len() > 120 {
                    format!("{}…", &snippet[..120])
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
            // Print the value for a specific key
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
            Err(e) => {
                print_error(format!("Invalid JSON params: {e}"));
                return Ok(());
            }
        };

    // Verify tool exists before showing spinner
    if !lab.tools().get(name).await {
        print_error(format!(
            "Tool '{name}' not found. Run `lab tools` to list available tools."
        ));
        return Ok(());
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
        print_error(err);
    }

    Ok(())
}

// ─── Agent helpers ───────────────────────────────────────────────

/// Parse all <tool_call>…</tool_call> blocks out of an LLM response.
fn extract_tool_calls(text: &str) -> Vec<serde_json::Value> {
    let mut calls = Vec::new();
    let open_tag = "<tool_call>";
    let close_tag = "</tool_call>";
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find(open_tag) {
        let content_start = cursor + start + open_tag.len();
        match text[content_start..].find(close_tag) {
            Some(end) => {
                let json_str = text[content_start..content_start + end].trim();
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    calls.push(val);
                }
                cursor = content_start + end + close_tag.len();
            }
            None => break,
        }
    }
    calls
}

/// Remove all <tool_call>…</tool_call> blocks; return the remaining text.
fn strip_tool_calls(text: &str) -> String {
    let open_tag = "<tool_call>";
    let close_tag = "</tool_call>";
    let mut result = String::new();
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find(open_tag) {
        result.push_str(&text[cursor..cursor + start]);
        let content_start = cursor + start + open_tag.len();
        match text[content_start..].find(close_tag) {
            Some(end) => cursor = content_start + end + close_tag.len(),
            None => {
                cursor = content_start;
                break;
            }
        }
    }
    result.push_str(&text[cursor..]);
    result.trim().to_string()
}

/// One-line summary of a tool result for the chat UI.
fn summarize_tool_result(tool_name: &str, data: &serde_json::Value) -> String {
    match tool_name {
        "read_file" => {
            let n = data
                .get("total_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!("{n} lines")
        }
        "list_directory" => {
            let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("{n} entries")
        }
        "glob_search" => {
            let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("{n} files")
        }
        "grep_search" => {
            let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("{n} matches")
        }
        "web_search" | "tavily_search" => {
            let n = data
                .get("results")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            format!("{n} results")
        }
        "fetch_url" => {
            let n = data.get("char_count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("{n} chars")
        }
        "bash" => {
            let rc = data.get("returncode").and_then(|v| v.as_i64()).unwrap_or(0);
            format!("exit {rc}")
        }
        "write_file" => {
            let n = data
                .get("bytes_written")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!("{n}B written")
        }
        "edit_file" => {
            let n = data
                .get("replacements_made")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!("{n} replacement(s)")
        }
        _ => "ok".to_string(),
    }
}

/// Hint string for each tool's parameters (used in system prompt).
fn tool_param_hints(name: &str) -> &'static str {
    match name {
        "read_file" => "path, offset?, limit?",
        "write_file" => "path, content",
        "edit_file" => "path, old_string, new_string, replace_all?",
        "list_directory" => "path?, limit?",
        "file_info" => "path",
        "glob_search" => "pattern, path?, limit?",
        "grep_search" => "pattern, path?, glob?, max_files?, context_lines?",
        "bash" => "command, timeout?",
        "git_status" => "",
        "web_search" => "query, max_results?",
        "tavily_search" => "query, max_results?, search_depth?, include_answer?",
        "fetch_url" => "url, max_chars?",
        _ => "...",
    }
}

/// Build the full agent system prompt with tool manifest.
fn build_agent_system_prompt(tools: &[serde_json::Value], config: &LabConfig) -> String {
    use std::collections::BTreeMap;

    let project_ctx = build_project_context(config);

    // Group tools by category
    let mut by_cat: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(|v: &serde_json::Value| v.as_str())
            .unwrap_or("?");
        let desc = tool
            .get("description")
            .and_then(|v: &serde_json::Value| v.as_str())
            .unwrap_or("");
        let cat = tool
            .get("category")
            .and_then(|v: &serde_json::Value| v.as_str())
            .unwrap_or("other");
        let params = tool_param_hints(name);
        let sig = if params.is_empty() {
            format!("{name}()\n  {desc}")
        } else {
            format!("{name}({params})\n  {desc}")
        };
        by_cat.entry(cat).or_default().push(sig);
    }

    let tool_manifest: String = by_cat
        .iter()
        .map(|(cat, sigs)| format!("### {}\n{}", capitalize(cat), sigs.join("\n\n")))
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        r#"You are an autonomous AI agent for the AI Research Lab — a Rust-based multi-agent research workspace.
You have full access to the filesystem, shell, and web. Use tools whenever you need information or need to take action.

## Workspace
{workspace}

{project_ctx}

## How to call tools

Output tool calls in this exact format (one per line, inside tags):
<tool_call>{{"tool": "TOOL_NAME", "params": {{"key": "value"}}}}</tool_call>

Rules:
- Call as many tools as needed before answering
- After receiving tool results, reason about them and call more tools if needed
- Give your final answer as plain text — no <tool_call> tags in the final answer
- Be concise. Use code blocks for code.

## Available Tools

{tool_manifest}

## Examples

User: list files in crates/lab-cli/src
<tool_call>{{"tool": "list_directory", "params": {{"path": "crates/lab-cli/src"}}}}</tool_call>

User: what does the ResearchLab struct do?
<tool_call>{{"tool": "grep_search", "params": {{"pattern": "pub struct ResearchLab", "glob": "*.rs"}}}}</tool_call>

User: latest rust async news
<tool_call>{{"tool": "web_search", "params": {{"query": "Rust async news 2025", "max_results": 5}}}}</tool_call>
"#,
        workspace = config.workspace.display(),
    )
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
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

pub(crate) async fn cmd_improve(
    pattern: &str,
    max_candidates: usize,
    max_apply: usize,
    dry_run: bool,
    run_tests: bool,
    create_branch: bool,
    web_research: bool,
) -> anyhow::Result<()> {
    use lab_agents::{AutonomousImprovementPipeline, ImprovementConfig};
    use lab_core::LabContainerBuilder;

    let config = LabConfig::default();

    println!();
    println!("{}", "Autonomous Self-Improvement".bright_white().bold());
    println!("{}", "Research → Plan → Implement → Verify loop.".dimmed());
    print_rule();
    print_meta_row("Pattern", pattern);
    print_meta_row("Max candidates", max_candidates);
    print_meta_row("Max to apply", if dry_run { 0 } else { max_apply });
    print_meta_row(
        "Mode",
        if dry_run {
            "Dry run".yellow().bold().to_string()
        } else {
            "Live".green().bold().to_string()
        },
    );
    print_meta_row("cargo test", if run_tests { "yes" } else { "no" });
    print_meta_row("Git branch", if create_branch { "yes" } else { "no" });
    print_meta_row("Web research", if web_research { "yes" } else { "no" });
    print_rule();

    if !config.llm_configured() {
        print_error(format!("LLM unavailable — {}.", llm_setup_hint(&config)));
        return Ok(());
    }

    let container = LabContainerBuilder::new(config.clone()).build();
    let mut registry = container.tool_registry;
    let mut memory = container.memory;
    let llm_box = container.llm_client;
    let llm_ref = match llm_box.as_deref() {
        Some(l) => l,
        None => {
            print_error("LLM client not initialized.");
            return Ok(());
        }
    };

    let session_id = format!("improve-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let improvement_config = ImprovementConfig {
        pattern: pattern.to_string(),
        max_candidates,
        max_per_run: max_apply,
        dry_run,
        run_tests,
        create_branch,
        web_research,
        use_clippy: true,
    };

    let mut pipeline = AutonomousImprovementPipeline::new(
        session_id,
        config.workspace.clone(),
        improvement_config,
    );

    // Phase display
    println!(
        "  {} {}",
        "Phase 1+2".bright_black(),
        "Researching and reviewing codebase…".dimmed()
    );

    let report = with_spinner(
        "Analysing → Planning → Applying",
        pipeline.run(&mut registry, &mut memory, llm_ref, &config.model),
    )
    .await;

    // ── Results ───────────────────────────────────────────────────
    println!();
    println!("{}", "Results".bright_white().bold());
    print_rule();

    if dry_run {
        println!(
            "  {} {} candidate(s) planned (dry run — not applied)",
            "→".bright_black(),
            report.total_planned.to_string().cyan().bold()
        );
        println!();
        for (i, attempt) in report.attempts.iter().enumerate() {
            let priority_str = match attempt.priority.as_str() {
                "high" => "HIGH  ".red().bold().to_string(),
                "medium" => "MEDIUM".yellow().bold().to_string(),
                _ => "LOW   ".bright_black().bold().to_string(),
            };
            println!(
                "  {}. [{}]  {}",
                (i + 1).to_string().bright_black(),
                priority_str,
                attempt.file.cyan()
            );
            println!("          {}", attempt.task.dimmed());
            println!();
        }
    } else {
        print_meta_row("Planned", report.total_planned);
        print_meta_row(
            "Applied",
            format!("{}", report.applied.to_string().green().bold()),
        );
        print_meta_row(
            "Failed",
            if report.failed > 0 {
                report.failed.to_string().red().bold().to_string()
            } else {
                "0".dimmed().to_string()
            },
        );
        if report.skipped > 0 {
            print_meta_row(
                "Remaining",
                format!("{} (run again to apply)", report.skipped),
            );
        }
        if let Some(ref branch) = report.git_branch {
            print_meta_row("Branch", branch.cyan());
        }
        print_meta_row("Duration", format!("{:.1}s", report.duration_secs));
        println!();

        // Attempt details
        for attempt in &report.attempts {
            let (icon, detail) = if attempt.success {
                (
                    "✓".green().bold().to_string(),
                    format!("{} edit(s)", attempt.edits_applied)
                        .bright_black()
                        .to_string(),
                )
            } else {
                let reason = attempt
                    .error
                    .as_deref()
                    .unwrap_or("unknown")
                    .chars()
                    .take(55)
                    .collect::<String>();
                ("✗".red().bold().to_string(), reason.yellow().to_string())
            };
            println!("  {}  {}  {}", icon, attempt.file.cyan(), detail);
            println!(
                "     {}",
                attempt.task.chars().take(72).collect::<String>().dimmed()
            );
            println!();
        }

        if report.applied > 0 {
            if let Some(ref branch) = report.git_branch {
                println!(
                    "  {} {}",
                    "Merge with:".bright_black(),
                    format!("git merge {branch}").cyan()
                );
            }
        } else if report.total_planned == 0 {
            print_warning("No candidates generated — check that LLM is configured correctly.");
        }
    }

    Ok(())
}

pub(crate) async fn cmd_self_edit(file: &str, task: &str, dry_run: bool) -> anyhow::Result<()> {
    use lab_agents::SelfEditAgent;
    use lab_core::config::AgentProfile;
    use lab_core::LabContainerBuilder;

    let config = LabConfig::default();

    println!();
    println!("{}", "Self-Edit".bright_white().bold());
    println!(
        "{}",
        "LLM-driven targeted file improvement with build verification.".dimmed()
    );
    print_rule();
    print_meta_row("File", file);
    print_meta_row("Task", task);
    print_meta_row(
        "Mode",
        if dry_run {
            "Dry run (no changes)".yellow().to_string()
        } else {
            "Live (apply + cargo check)".green().to_string()
        },
    );
    print_rule();

    if !config.llm_configured() {
        print_error(format!("LLM unavailable — {}.", llm_setup_hint(&config)));
        return Ok(());
    }

    let container = LabContainerBuilder::new(config.clone()).build();
    let mut registry = container.tool_registry;
    let mut memory = container.memory;
    let llm_box = container.llm_client;
    let llm_ref = match llm_box.as_deref() {
        Some(l) => l,
        None => {
            print_error("LLM client not initialized.");
            return Ok(());
        }
    };

    let session_id = format!("self-edit-{}", &uuid::Uuid::new_v4().to_string()[..6]);
    let agent_id = format!("self-edit-agent-{}", &uuid::Uuid::new_v4().to_string()[..6]);

    let mut agent = SelfEditAgent::new(
        agent_id,
        session_id,
        AgentProfile::default(),
        config.workspace.clone(),
    );

    let result = with_spinner(
        if dry_run {
            "Analysing file"
        } else {
            "Editing + verifying"
        },
        agent.execute(
            &mut registry,
            &mut memory,
            file,
            task,
            dry_run,
            llm_ref,
            &config.model,
        ),
    )
    .await;

    println!();

    if result.success {
        let data = &result.data;

        let proposed = data
            .get("edits_proposed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let applied = data
            .get("edits_applied")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        if dry_run {
            print_success(format!(
                "{proposed} edit(s) proposed (dry run — not applied)"
            ));
            println!();
            if let Some(edits) = data.get("edits").and_then(|v| v.as_array()) {
                for (i, edit) in edits.iter().enumerate() {
                    let old = edit.get("old").and_then(|v| v.as_str()).unwrap_or("");
                    let new = edit.get("new").and_then(|v| v.as_str()).unwrap_or("");
                    println!(
                        "  {} {}",
                        format!("Edit {}", i + 1).cyan().bold(),
                        format!("({} → {} chars)", old.len(), new.len()).bright_black()
                    );
                    // Show first line of old/new as preview
                    let old_preview = old.lines().next().unwrap_or("").trim();
                    let new_preview = new.lines().next().unwrap_or("").trim();
                    println!("    {} {}", "─".bright_black(), old_preview.red().dimmed());
                    println!(
                        "    {} {}",
                        "+".bright_black(),
                        new_preview.green().dimmed()
                    );
                    println!();
                }
            }
        } else if proposed == 0 {
            print_meta_row("Result", "No changes needed".dimmed());
        } else {
            print_success(format!("{applied}/{proposed} edit(s) applied"));
            print_meta_row("Verified", "cargo check passed".green());
            if let Some(out) = data
                .get("compiler_output")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                println!();
                for line in out.lines().take(8) {
                    println!("  {}", line.dimmed());
                }
            }
        }
    } else {
        let err = result.error.as_deref().unwrap_or("unknown error");
        print_error(err);

        if let Some(compiler_errors) = result
            .data
            .get("compiler_errors")
            .and_then(|v| v.as_str())
            .filter(|s: &&str| !s.is_empty())
        {
            println!();
            println!("{}", "Compiler output:".bright_black());
            for line in compiler_errors.lines().take(20) {
                println!("  {}", line.yellow());
            }
        }
    }

    Ok(())
}

pub(crate) async fn cmd_report(open: bool, output: Option<&str>) -> anyhow::Result<()> {
    let config = LabConfig::default();
    let outputs_dir = config.full_path(&config.outputs_dir);

    // Resolve the HTML report path
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

fn session_record_count(path: &Path) -> usize {
    if path.exists() {
        std::fs::read_dir(path)
            .map(|entries| entries.count())
            .unwrap_or(0)
    } else {
        0
    }
}
