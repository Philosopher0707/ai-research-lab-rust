use crate::ui::{print_meta_row, print_rule, with_spinner};
use colored::Colorize;
use lab_core::LabConfig;
use std::path::{Path, PathBuf};

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

pub(crate) fn build_cli_pipeline_request(
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

pub(crate) fn print_pipeline_results(result: &lab_pipelines::PipelineResult, output_path: &Path) {
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
