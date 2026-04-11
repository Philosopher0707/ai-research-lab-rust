# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Verification

```bash
# Full workspace build
cargo build --release

# Build only the CLI binary
cargo build -p lab-cli --release

# Run all tests
cargo test --workspace -- --nocapture

# Run a single crate's tests
cargo test -p lab-api --tests -- --nocapture

# Run a single test by name
cargo test -p lab-api --tests runtime_endpoints -- --nocapture

# Lint
cargo clippy --workspace

# Format check
cargo fmt --check
```

The release binary is `./target/release/lab`. For development use `cargo run -p lab-cli --bin lab -- <subcommand>`.

Set `NO_SPINNER=1` when capturing CLI output in scripts or tests — the spinner writes raw ANSI escape sequences that corrupt piped output.

Set `LAB_DEBUG=1` to enable debug tracing. Default log level is WARN to keep CLI output clean.

## Provider Configuration

`.env` is loaded automatically from the workspace root at startup via `LabConfig::initialize_process_env()` (uses `Once` so it only runs once per process). The `.env.example` shows all variables. Key ones:

```env
LAB_PROVIDER=anthropic          # anthropic | openai | openrouter | deepseek | local | ...
LAB_MODEL=claude-haiku-4-5      # overrides the provider default
ANTHROPIC_API_KEY=sk-ant-...
```

Provider is auto-detected from whichever `*_API_KEY` variable is set if `LAB_PROVIDER` is unset. `local` mode requires no API key — set `LAB_BASE_URL` to an OpenAI-compatible endpoint.

## Architecture

### Crate Dependency Order

```
lab-core  ←  lab-memory
          ←  lab-permissions
          ←  lab-tools
          ←  lab-agents   ←  lab-pipelines
          ←  lab-reports
                         ←  lab-api
                         ←  lab-cli
```

All crates share versions and third-party dependencies through the workspace `Cargo.toml`. Add new deps there, then reference with `.workspace = true` in the crate.

### lab-core

The hub everything else imports. Key types:

- **`LabConfig`** (`config.rs`) — loaded via `LabConfig::default()` or `LabConfig::with_workspace(path)`. Config priority: explicit env vars (`LAB_*`) > `.env.local` > `.env.<profile>` > `.env` > TOML/YAML config files > compiled defaults.
- **`providers.rs`** — single source of truth for all LLM provider metadata (`ProviderDef`, `PROVIDERS` const, `find(id)`, `detect_from_env()`). Do not add provider-specific logic in config.rs or client.rs — update this table instead.
- **`LabContainerBuilder`** (`container.rs`) — dependency-injection builder. Constructs `LabContainer` with `ToolRegistry`, `MemoryWorkspace`, `LLMClient`, `SessionStore`, `Arc<EventBus>`, `PermissionEngine`. Always use the builder rather than constructing these individually.
- **`ResearchLab`** (`engine.rs`) — master orchestrator. Holds all runtime state. Created via `ResearchLab::new(config)` (calls the container builder internally). `event_bus_arc()` returns a cloned `Arc<EventBus>` — used by the API server to share the bus without locking the lab.
- **`LLMClient`** trait (`llm/client.rs`) — `chat(messages, model, temperature, max_tokens) -> Result<ChatResponse, LLMError>`. Typed error: match `LLMError::RateLimited` to retry, `LLMError::NotConfigured` to bail early. Implementations: `AnthropicClient`, `OpenAICompatibleClient`. Created by `lab_core::llm::create_client(provider, key, model, base_url)`.
- **`EventBus`** (`events/bus.rs`) — broadcast-based, wrapped in `Arc`. `emit(&self)` is non-mutating — no exclusive lock needed. `subscribe()` returns `broadcast::Receiver<LabEvent>`. History is a fixed-size ring buffer accessible via `get_history(filter, limit)`.
- **`ChatMessage::system()`** / **`ChatMessage::user()`** — always include a system message in agent LLM calls.

### lab-agents

Five concrete agents plus orchestration infrastructure:

- **`ResearcherAgent`** — scans files matching a glob, builds a structural map
- **`ReviewerAgent`** — currently regex-based pattern matching (planned replacement: clippy JSON)
- **`CoderAgent`** — generates or edits code files via LLM
- **`SummarizerAgent`** — consolidates memory entries into a report
- **`SelfEditAgent`** (`self_edit.rs`) — LLM-driven single-file editing: reads file → LLM outputs `<edits>[{old, new}]</edits>` → applies via `edit_file` tool → `cargo check` → revert on failure
- **`AutonomousImprovementPipeline`** (`autonomous_improvement.rs`) — full loop: research → review → plan → edit per file (grouped) → verify → commit

`AgentFactory` / `AgentFactoryRegistry` in `runtime.rs` provide the plugin pattern for registering new agent types. The shared `execute_agent_with_registry()` function is the single dispatch point used by both CLI and API.

All agent LLM calls go through `ask_agent_llm()` in `lib.rs` — pass a system prompt as `Some(&str)`.

### lab-pipelines

Sequential stage executor: `discover → research → analyze → review → code → summarize → report`. Delegates each stage to `MultiAgentCollaborator` from `lab-agents`. Writes `lab-outputs/pipeline-report.md` and `lab-outputs/pipeline-report.html`.

### lab-api

Axum server. `AppState` has two fields:
- `lab: RwLock<ResearchLab>` — handlers use `state.lab.read().await` or `state.lab.write().await`
- `events: Arc<EventBus>` — the lab's event bus, shared via `lab.event_bus_arc()` during construction. WebSocket handlers subscribe directly (`state.events.subscribe()`) without locking the lab. The `event_history` endpoint also reads from `state.events` — no lock needed.

Routes defined in `router.rs`, handlers in `handlers.rs`.

Integration tests in `crates/lab-api/tests/` use a `TestServer` helper (`tests/support/mod.rs`) that binds to port 0 (random) — safe for parallel test runs.

### lab-tools

`ToolRegistry` holds `Box<dyn Tool>`. Built-in tools registered at startup: `read_file`, `write_file`, `edit_file`, `list_directory`, `bash`, `web_search`, `tavily_search`, `fetch_url`, `git_status`, `file_info`, `glob_files`. The `edit_file` tool returns an error (not success) when `old_string` is not found in the file.

### lab-memory

`MemoryWorkspace` is a key-value store scoped by session ID, backed by the filesystem at `lab-memory/<session_id>/index.json`. Access via `memory.get(session_id, key)` / `memory.set(session_id, key, value, tags)`.

## Autonomous Self-Improvement Pipeline

`lab improve` runs the full loop. Important flags:

```bash
lab improve --dry-run --max-candidates 5   # plan only, no edits
lab improve --max-candidates 10 --max-apply 3 --no-branch  # apply 3, no git branch
lab improve --run-tests  # also run cargo test after each edit
```

Candidates targeting the **same file** are grouped into a single `SelfEditAgent` call to prevent sequential edits on the same file from stomping each other.

`commit_change()` uses `git add -- <specific_file>` (not `git add -A`) to stage only the edited file.

## Structured Output Protocols

LLM responses use XML-tag-delimited JSON (not native function calling):

- `<edits>[{"old": "...", "new": "..."}]</edits>` — SelfEditAgent edit instructions
- `<candidates>[{"file": "...", "task": "...", "priority": "high|medium|low", "rationale": "..."}]</candidates>` — ImprovementPlanner candidate list
- `<tool_call>{"name": "...", "params": {...}}</tool_call>` — ReAct-style tool invocations in chat mode

## In-Progress: Industry-Level Improvement Architecture

The current `ReviewerAgent` (regex) and `SelfEditAgent` (exact string match) are being replaced with:

1. **`improvement/clippy_runner.rs`** — parse `cargo clippy --message-format json` into `Vec<ClippyFinding>` with exact byte offsets and `SuggestionApplicability`
2. **`improvement/risk.rs`** — classify each finding: `AutoFix | Patch | LLMEdit | HumanReview`
3. **`improvement/edit_strategy.rs`** — `EditStrategy` trait with three impls:
   - `ClippyFixStrategy` — `cargo clippy --fix -- -A clippy::all -W <lint>` (no LLM)
   - `DiffPatchStrategy` — LLM outputs unified diff, applied via `diffy` crate
   - `LLMEditStrategy` — LLM replaces compiler-provided byte range (no "guess the old string")
4. **`improvement/verification.rs`** — `VerificationChain`: rustfmt (corrective) → clippy (vs baseline) → cargo check → cargo test

The `ImprovementReport` / `ImprovementConfig` struct fields must remain stable — `commands.rs` in lab-cli depends on them.
