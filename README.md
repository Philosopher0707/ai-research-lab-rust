# AI Research Lab (Rust)

Production-oriented Rust workspace for interactive codebase research, multi-agent execution, staged analysis pipelines, and an HTTP API over the same runtime.

## Status

Current release target: `0.2.0`

What is ready now:

- interactive CLI with a cleaner production shell
- provider-aware `.env` loading from the workspace
- OpenAI-compatible local provider support without a fake API key
- unified research pipeline runtime used by CLI and API
- shared agent execution runtime used by CLI and API
- REST API with session, memory, agent, ask, workflow, and pipeline routes
- end-to-end tested `/pipelines/run` API path

## Workspace Layout

- [crates/lab-cli](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-cli) — terminal interface and command dispatch
- [crates/lab-api](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-api) — HTTP and WebSocket server
- [crates/lab-core](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-core) — config, sessions, events, workflows, orchestration
- [crates/lab-agents](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-agents) — researcher, reviewer, coder, summarizer, shared agent runtime
- [crates/lab-pipelines](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-pipelines) — staged research pipeline executor
- [crates/lab-memory](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-memory) — persistent workspace memory
- [crates/lab-tools](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-tools) — built-in filesystem and shell tools
- [crates/lab-reports](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-reports) — HTML reporting helpers

## Quick Start

### 1. Configure a provider

Copy the example env file:

```bash
cp .env.example .env
```

Then either:

- run `lab setup`
- or edit `.env` directly

For a local OpenAI-compatible server:

```env
LAB_PROVIDER=local
LAB_BASE_URL=http://localhost:11434/v1
LAB_MODEL=llama3.2
```

No fake key is required in local mode.

### 2. Run the CLI

```bash
cargo run -p lab-cli --bin lab
```

Useful commands:

- `lab`
- `lab status`
- `lab pipeline --pattern '**/*.rs'`
- `lab agent --type reviewer --pattern '**/*.rs'`
- `lab serve --port 8000`

### 3. Run the API

```bash
cargo run -p lab-cli --bin lab -- serve --port 8000
```

Important routes:

- `GET /health`
- `POST /sessions`
- `POST /ask`
- `POST /agents/run`
- `POST /pipelines/run`
- `GET /events`

## Architecture Maps

- [RUNTIME_MAP.md](/Users/philosopher/archived/ai-research-lab-rust/RUNTIME_MAP.md) — implementation-accurate runtime map
- [DESIGN.md](/Users/philosopher/archived/ai-research-lab-rust/DESIGN.md) — parity design and roadmap
- [CHANGELOG.md](/Users/philosopher/archived/ai-research-lab-rust/CHANGELOG.md) — release history
- [RELEASE.md](/Users/philosopher/archived/ai-research-lab-rust/RELEASE.md) — release process and checklist

## Verification

Primary verification command:

```bash
cargo test --workspace -- --nocapture
```

Focused checks:

```bash
cargo build -p lab-cli
cargo test -p lab-api --tests -- --nocapture
```

## Notes

- `.env` is loaded automatically from the workspace root.
- `lab pipeline` and `POST /pipelines/run` now use the same pipeline engine.
- `lab agent` and `POST /agents/run` now use the same shared agent runtime.
- Some internal crates may still have future-facing roadmap surface, but the current CLI/API/runtime path is coherent and tested.
