use crate::ui::{
    llm_setup_hint, print_chat_header, print_error, print_help_menu, print_success, print_warning,
    with_spinner,
};
use colored::Colorize;
use lab_core::llm::ChatMessage;
use lab_core::{LabConfig, ResearchLab};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::collections::BTreeMap;
use std::io::Write as _;

/// Rustyline prompt: ANSI sequences wrapped in \x01/\x02 so readline
/// correctly measures the visible width and positions the cursor.
const PROMPT: &str = "\x01\x1b[36;1m\x02lab \u{203a} \x01\x1b[0m\x02";

pub(crate) async fn cmd_chat() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let lab = ResearchLab::new(config.clone());
    with_spinner("Booting workspace", lab.start()).await?;
    print_chat_header(&config, lab.has_llm());

    let tool_list = lab.tools().list_tools().await;
    let system_prompt = build_agent_system_prompt(&tool_list, &config);
    let mut history = vec![ChatMessage::system(&system_prompt)];

    // Persist REPL history across sessions
    let history_path = config.full_path(&config.sessions_dir).join(".repl_history");
    if let Some(parent) = history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut rl = DefaultEditor::new()?;
    let _ = rl.load_history(&history_path);

    loop {
        let input = match rl.readline(PROMPT) {
            Ok(line) => {
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() {
                    let _ = rl.add_history_entry(&trimmed);
                }
                trimmed
            }
            // Ctrl+C or Ctrl+D → clean exit
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                println!();
                println!("{}", "Session closed.".dimmed());
                break;
            }
            Err(e) => {
                eprintln!("Input error: {e}");
                break;
            }
        };
        if input.is_empty() {
            continue;
        }

        // ── Slash commands ────────────────────────────────────────
        if input == "/exit" || input == "/quit" || input == "/q" {
            println!();
            println!("{}", "Session closed.".dimmed());
            break;
        } else if input == "/help" || input == "/?" {
            print_help_menu();
            continue;
        } else if input == "/status" {
            let _ = super::cmd_status().await;
            println!();
            continue;
        } else if input == "/pipeline" {
            let _ = super::cmd_pipeline_run("**/*.rs", None, false, true).await;
            println!();
            continue;
        } else if input == "/clear" {
            history.truncate(1);
            print_success("Conversation cleared.");
            println!();
            continue;
        } else if input == "/tools" {
            let _ = super::cmd_tools().await;
            println!();
            continue;
        } else if input == "/report" {
            let _ = super::cmd_report(true, None).await;
            println!();
            continue;
        } else if input == "/memory" {
            let _ = super::cmd_memory(None, None).await;
            println!();
            continue;
        } else if let Some(query) = input.strip_prefix("/search ").map(str::trim) {
            if query.is_empty() {
                print_warning("Usage: /search <query>");
            } else {
                let _ = super::cmd_search(query, 5).await;
            }
            println!();
            continue;
        } else if let Some(url) = input.strip_prefix("/fetch ").map(str::trim) {
            if url.is_empty() {
                print_warning("Usage: /fetch <url>");
            } else {
                let _ = super::cmd_tool("fetch_url", &format!(r#"{{"url":"{url}"}}"#)).await;
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
        // ─────────────────────────────────────────────────────────

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

            history.push(ChatMessage::assistant(&response));

            let preamble = strip_tool_calls(&response);
            if !preamble.is_empty() {
                println!();
                println!("{}", preamble.trim().dimmed());
            }
            println!();

            let mut result_parts: Vec<String> = Vec::new();
            for call in &tool_calls {
                let tool_name = call.get("tool").and_then(|v| v.as_str()).unwrap_or("?");
                let params: std::collections::HashMap<String, serde_json::Value> = call
                    .get("params")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();

                let params_json =
                    serde_json::to_string(&call.get("params").cloned().unwrap_or_default())
                        .unwrap_or_default();
                let preview = if params_json.len() > 55 {
                    let end = params_json.floor_char_boundary(55);
                    format!("{}…", &params_json[..end])
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

    let _ = rl.save_history(&history_path);
    lab.shutdown().await?;
    Ok(())
}

// ─── LLM response helpers ─────────────────────────────────────────

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

// ─── System prompt builders ───────────────────────────────────────

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

fn build_agent_system_prompt(tools: &[serde_json::Value], config: &LabConfig) -> String {
    let project_ctx = build_project_context(config);

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
