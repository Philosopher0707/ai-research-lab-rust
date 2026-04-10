use crate::ui::{print_meta_row, print_rule};
use colored::Colorize;
use lab_core::LabConfig;
use std::io::Write as _;
use std::path::Path;

#[derive(Clone, Copy)]
struct Provider {
    id: &'static str,
    name: &'static str,
    env_key: &'static str,
    key_url: &'static str,
    desc: &'static str,
    models: &'static [(&'static str, &'static str)],
    extra: Option<(&'static str, &'static str, &'static str)>,
}

const PROVIDERS: &[Provider] = &[
    Provider {
        id: "anthropic",
        name: "Anthropic",
        env_key: "ANTHROPIC_API_KEY",
        key_url: "console.anthropic.com/settings/keys",
        desc: "Claude — best for code & reasoning",
        models: &[
            ("claude-sonnet-4-6", "Sonnet 4.6  — recommended (balanced)"),
            ("claude-opus-4", "Opus 4      — most capable, slower"),
            ("claude-haiku-4-5", "Haiku 4.5   — fastest, cheapest"),
        ],
        extra: None,
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        env_key: "OPENAI_API_KEY",
        key_url: "platform.openai.com/api-keys",
        desc: "GPT-4o, o3-mini — widely supported",
        models: &[
            ("gpt-4o", "GPT-4o       — flagship (recommended)"),
            ("gpt-4o-mini", "GPT-4o mini  — fast & cheap"),
            ("o3-mini", "o3-mini      — best reasoning"),
            ("o1-preview", "o1-preview   — advanced reasoning"),
        ],
        extra: None,
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        env_key: "OPENROUTER_API_KEY",
        key_url: "openrouter.ai/settings/keys",
        desc: "300+ models — Claude, Llama, Gemini, Qwen...",
        models: &[
            ("anthropic/claude-sonnet-4-5", "Claude Sonnet 4.5"),
            (
                "anthropic/claude-opus-4",
                "Claude Opus 4     (most capable)",
            ),
            (
                "meta-llama/llama-3.3-70b-instruct:free",
                "Llama 3.3 70B     FREE",
            ),
            ("qwen/qwen3-235b-a22b:free", "Qwen3 235B        FREE"),
            ("google/gemini-2.0-flash-exp:free", "Gemini 2.0 Flash  FREE"),
            ("deepseek/deepseek-chat", "DeepSeek Chat     via OpenRouter"),
        ],
        extra: None,
    },
    Provider {
        id: "deepseek",
        name: "DeepSeek",
        env_key: "DEEPSEEK_API_KEY",
        key_url: "platform.deepseek.com/api_keys",
        desc: "Very cheap (~$0.14/M tokens), strong at code",
        models: &[
            (
                "deepseek-chat",
                "deepseek-chat      — fast, general purpose",
            ),
            (
                "deepseek-reasoner",
                "deepseek-reasoner  — R1, deep thinking",
            ),
            ("deepseek-coder", "deepseek-coder     — code specialist"),
        ],
        extra: None,
    },
    Provider {
        id: "zhipu",
        name: "ZhipuAI (智谱AI)",
        env_key: "ZHIPU_API_KEY",
        key_url: "open.bigmodel.cn/usercenter/apikeys",
        desc: "GLM series — glm-4-flash has a free tier",
        models: &[
            ("glm-4-flash", "GLM-4-Flash  — FREE tier, fast"),
            ("glm-4-plus", "GLM-4-Plus   — high performance"),
            ("glm-4", "GLM-4        — standard"),
            ("glm-4-long", "GLM-4-Long   — 128K context"),
        ],
        extra: None,
    },
    Provider {
        id: "minimax",
        name: "MiniMax",
        env_key: "MINIMAX_API_KEY",
        key_url: "platform.minimax.chat",
        desc: "MiniMax-Text-01, abab6.5s — requires Group ID",
        models: &[
            ("MiniMax-Text-01", "MiniMax-Text-01  — latest"),
            ("abab6.5s-chat", "ABAB 6.5S        — fast"),
            ("abab6.5-chat", "ABAB 6.5         — standard"),
        ],
        extra: Some(("MINIMAX_GROUP_ID", "Group ID (from platform dashboard)", "")),
    },
    Provider {
        id: "xai",
        name: "xAI  (Grok)",
        env_key: "XAI_API_KEY",
        key_url: "console.x.ai",
        desc: "Grok models — console.x.ai",
        models: &[
            ("grok-2", "Grok 2      — latest"),
            ("grok-beta", "Grok Beta   — previous gen"),
            ("grok-2-mini", "Grok 2 Mini — faster"),
        ],
        extra: None,
    },
    Provider {
        id: "local",
        name: "Local  (Ollama / LM Studio / vLLM)",
        env_key: "",
        key_url: "",
        desc: "Any OpenAI-compatible local server",
        models: &[
            ("llama3.2", "Llama 3.2      (ollama pull llama3.2)"),
            (
                "qwen2.5-coder",
                "Qwen2.5-Coder  (ollama pull qwen2.5-coder)",
            ),
            ("mistral", "Mistral        (ollama pull mistral)"),
            ("phi4", "Phi-4          (ollama pull phi4)"),
        ],
        extra: Some(("LAB_BASE_URL", "Server URL", "http://localhost:11434/v1")),
    },
];

pub(crate) async fn cmd_setup() -> anyhow::Result<()> {
    let config = LabConfig::default();
    let env_path = config.workspace.join(".env");

    println!();
    println!("{}", "AI Research Lab Setup".bright_white().bold());
    println!(
        "{}",
        "Save provider settings to .env for automatic loading in future lab runs.".dimmed()
    );
    print_rule();
    print_meta_row("Workspace", config.workspace.display());
    print_meta_row("Env File", env_path.display());
    print_rule();
    println!();

    println!("{}", "  Step 1 — Choose a provider:".bold());
    println!();
    for (index, provider) in PROVIDERS.iter().enumerate() {
        println!(
            "    {}  {:<35}  {}",
            format!("{:>2})", index + 1).cyan().bold(),
            provider.name.bold(),
            provider.desc.dimmed(),
        );
    }
    println!();

    let provider = loop {
        let raw = read_line(&format!("  Provider [1-{}]: ", PROVIDERS.len()));
        if raw.is_empty() {
            println!("{}", "  Setup cancelled.".dimmed());
            return Ok(());
        }

        if let Ok(choice) = raw.parse::<usize>() {
            if (1..=PROVIDERS.len()).contains(&choice) {
                break PROVIDERS[choice - 1];
            }
        }

        eprintln!(
            "  {} Please enter a number between 1 and {}",
            "✗".red(),
            PROVIDERS.len()
        );
    };

    println!();
    println!("  Provider: {}", provider.name.green().bold());
    println!();

    println!("{}", "  Step 2 — Choose a model:".bold());
    println!();
    for (index, (model_id, desc)) in provider.models.iter().enumerate() {
        println!(
            "    {}  {:<42}  {}",
            format!("{:>2})", index + 1).cyan().bold(),
            model_id.bold(),
            desc.dimmed(),
        );
    }
    println!(
        "    {}  {}",
        format!("{:>2})", provider.models.len() + 1).cyan().bold(),
        "Enter a custom model ID".dimmed(),
    );
    println!();

    let model = loop {
        let raw = read_line(&format!(
            "  Model [1-{}, or press Enter for default]: ",
            provider.models.len() + 1
        ));
        if raw.is_empty() {
            break provider.models[0].0.to_string();
        }

        if let Ok(choice) = raw.parse::<usize>() {
            if (1..=provider.models.len()).contains(&choice) {
                break provider.models[choice - 1].0.to_string();
            }

            if choice == provider.models.len() + 1 {
                let custom = read_line("  Custom model ID: ");
                if !custom.is_empty() {
                    break custom;
                }
                eprintln!("  {} Model ID cannot be empty", "✗".red());
                continue;
            }
        }

        eprintln!("  {} Enter a valid number", "✗".red());
    };

    println!();
    println!("  Model: {}", model.green().bold());
    println!();

    let api_key = if provider.id == "local" {
        println!("  {} Local server — no API key needed.", "".dimmed());
        String::new()
    } else {
        println!("{}", "  Step 3 — API key:".bold());
        let existing = std::env::var(provider.env_key).unwrap_or_default();
        if !existing.is_empty() {
            println!(
                "  {} is already set in your environment ({})",
                provider.env_key.bold(),
                mask_key(&existing).dimmed(),
            );
            let answer = read_line("  Save this key to .env? [Y/n]: ");
            if answer.to_lowercase() == "n" {
                let replacement = read_line(&format!(
                    "  Paste new {} (Enter to skip): ",
                    provider.env_key
                ));
                if replacement.is_empty() {
                    existing
                } else {
                    replacement
                }
            } else {
                existing
            }
        } else {
            println!(
                "  Get your key from: {}",
                provider.key_url.cyan().underline(),
            );
            let key = read_line(&format!("  {} (paste, then Enter): ", provider.env_key));
            if key.is_empty() {
                println!(
                    "{}",
                    "  Skipping key — you can add it later to .env".yellow()
                );
            }
            key
        }
    };

    let mut extras = Vec::new();
    if let Some((extra_env, label, default_val)) = provider.extra {
        println!();
        println!("{}", format!("  Step 4 — {}:", label).bold());
        let existing = std::env::var(extra_env).unwrap_or(default_val.to_string());
        let hint = if existing.is_empty() {
            String::new()
        } else {
            format!(" [{}]", existing.dimmed())
        };
        let value = read_line(&format!("  {}{}: ", extra_env, hint));
        let value = if value.is_empty() { existing } else { value };
        if !value.is_empty() {
            extras.push((extra_env.to_string(), value));
        }
    }

    println!();
    let mut updates = vec![
        ("LAB_PROVIDER".to_string(), provider.id.to_string()),
        ("LAB_MODEL".to_string(), model.clone()),
    ];
    if !api_key.is_empty() && !provider.env_key.is_empty() {
        updates.push((provider.env_key.to_string(), api_key.clone()));
    }
    updates.extend(extras);

    match write_env_updates(&env_path, &updates) {
        Ok(()) => println!(
            "  {} Config written to {}",
            "✓".green().bold(),
            env_path.display().to_string().dimmed(),
        ),
        Err(error) => eprintln!("  {} Could not write .env: {}", "✗".red(), error),
    }

    println!();
    println!("{}", "  ══ Summary ══════════════════════════════".dimmed());
    println!("  Provider : {}", provider.name.green().bold());
    println!("  Model    : {}", model.green().bold());
    if !api_key.is_empty() {
        println!("  Key      : {}", mask_key(&api_key).dimmed());
    }
    println!();
    println!(
        "  {}",
        "This workspace now auto-loads `.env` when you run `lab` here.".dimmed()
    );
    println!();
    println!("{}", "  Then try:".bold());
    println!("    {}  — interactive chat", "lab".cyan());
    println!("    {}  — analyse codebase", "lab pipeline".cyan());
    println!("    {}  — REST API on :8000", "lab serve".cyan());
    println!();

    Ok(())
}

fn read_line(prompt: &str) -> String {
    print!("{}", prompt.cyan());
    let _ = std::io::stdout().flush();
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer).unwrap_or(0);
    buffer.trim().to_string()
}

fn mask_key(key: &str) -> String {
    if key.len() <= 12 {
        return "***".to_string();
    }
    format!("{}...{}", &key[..6], &key[key.len() - 4..])
}

fn write_env_updates(path: &Path, updates: &[(String, String)]) -> anyhow::Result<()> {
    let content = if path.exists() {
        std::fs::read_to_string(path)?
    } else {
        String::new()
    };

    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    for (key, value) in updates {
        let prefix = format!("{key}=");
        let commented = format!("# {key}=");
        let new_line = format!("{key}={value}");

        if let Some(position) = lines
            .iter()
            .position(|line| line.starts_with(&prefix) || line.starts_with(&commented))
        {
            lines[position] = new_line;
        } else {
            lines.push(new_line);
        }
    }

    std::fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}
