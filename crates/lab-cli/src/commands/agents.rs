use crate::ui::{
    llm_setup_hint, print_meta_row, print_rule, print_success, print_warning, with_spinner,
};
use colored::Colorize;
use lab_core::config::AgentProfile;
use lab_core::{LabConfig, LabContainerBuilder};

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
            Err(e) => anyhow::bail!("{e}"),
        };

    if !result.success {
        let err = result.error.as_deref().unwrap_or("unknown error");
        anyhow::bail!("Agent failed: {err}");
    }

    println!();
    println!("{}", "Agent Output".bright_white().bold());
    print_rule();

    let data = &result.data;

    // Summary — present in SummarizerAgent
    if let Some(summary) = data.get("summary").and_then(|v| v.as_str()) {
        println!("{summary}");
        println!();
    }

    // File stats — present in ResearcherAgent
    if let Some(files) = data.get("files_analyzed").and_then(|v| v.as_u64()) {
        print_meta_row("Files analyzed", files);
    }

    // Issues / findings — present in ReviewerAgent
    if let Some(issues) = data.get("issues").and_then(|v| v.as_array()) {
        if issues.is_empty() {
            print_success("No issues found.");
        } else {
            println!();
            println!(
                "  {} {} issue(s)",
                "Found".bright_black(),
                issues.len().to_string().yellow().bold()
            );
            println!();
            for issue in issues.iter().take(20) {
                let msg = issue.get("message").and_then(|v| v.as_str()).unwrap_or(
                    issue
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                );
                let file = issue
                    .get("file")
                    .or_else(|| issue.get("path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let severity = issue.get("severity").and_then(|v| v.as_str()).unwrap_or("");
                let prefix = match severity {
                    "high" | "error" => "✗".red().bold().to_string(),
                    "medium" | "warning" => "⚠".yellow().bold().to_string(),
                    _ => "·".bright_black().to_string(),
                };
                if !msg.is_empty() {
                    println!("  {}  {}", prefix, msg);
                    if !file.is_empty() {
                        println!("     {}", file.dimmed());
                    }
                }
            }
            if issues.len() > 20 {
                println!("  {} {} more…", "…".bright_black(), issues.len() - 20);
            }
        }
        println!();
    }

    // Key findings — present in SummarizerAgent
    if let Some(findings) = data.get("key_findings").and_then(|v| v.as_array()) {
        if !findings.is_empty() {
            println!("{}", "Key findings".bright_black());
            for f in findings.iter().take(10) {
                let text = f.as_str().unwrap_or_default();
                if !text.is_empty() {
                    println!("  {}  {}", "·".bright_black(), text);
                }
            }
            println!();
        }
    }

    // Scalar fields — show whatever is left that isn't already printed
    let skip = ["summary", "issues", "key_findings", "files_analyzed"];
    let mut extras: Vec<(&str, String)> = data
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| !skip.contains(&k.as_str()))
        .filter_map(|(k, v)| {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => return None,
            };
            Some((k.as_str(), s))
        })
        .collect();
    extras.sort_by_key(|(k, _)| *k);
    for (k, v) in extras.iter().take(8) {
        print_meta_row(k, v);
    }

    Ok(())
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
        anyhow::bail!("LLM unavailable — {}.", llm_setup_hint(&config));
    }

    let container = LabContainerBuilder::new(config.clone()).build();
    let mut registry = container.tool_registry;
    let mut memory = container.memory;
    let llm_box = container.llm_client;
    let llm_ref = match llm_box.as_deref() {
        Some(l) => l,
        None => anyhow::bail!("LLM client not initialized."),
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
        anyhow::bail!("LLM unavailable — {}.", llm_setup_hint(&config));
    }

    let container = LabContainerBuilder::new(config.clone()).build();
    let mut registry = container.tool_registry;
    let mut memory = container.memory;
    let llm_box = container.llm_client;
    let llm_ref = match llm_box.as_deref() {
        Some(l) => l,
        None => anyhow::bail!("LLM client not initialized."),
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
            println!();
        }
        anyhow::bail!("{err}");
    }

    Ok(())
}
