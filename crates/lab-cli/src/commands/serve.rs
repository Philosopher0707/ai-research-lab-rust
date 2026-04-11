use crate::ui::{print_meta_row, print_rule, print_success, provider_display_name};
use colored::Colorize;
use lab_core::LabConfig;
use std::sync::Arc;

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
    print_success(format!("Serving on {addr}  ·  Ctrl+C to stop"));
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    eprintln!();
    print_success("Server stopped.");
    Ok(())
}
