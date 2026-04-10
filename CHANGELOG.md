# Changelog

All notable changes to this project will be documented in this file.

## 0.2.0 - 2026-04-10

### Added

- implementation-accurate runtime map in [RUNTIME_MAP.md](/Users/philosopher/archived/ai-research-lab-rust/RUNTIME_MAP.md)
- modular API structure with dedicated state, models, handlers, router, and integration tests
- modular CLI structure with dedicated UI, command, and setup modules
- shared agent execution runtime in [runtime.rs](/Users/philosopher/archived/ai-research-lab-rust/crates/lab-agents/src/runtime.rs)
- end-to-end API integration test for `POST /pipelines/run`
- top-level release docs: `README.md`, `CHANGELOG.md`, and `RELEASE.md`

### Changed

- upgraded workspace release target from `0.1.0` to `0.2.0`
- improved CLI presentation with a red production-style ASCII header
- unified pipeline execution so CLI and API both run through the same pipeline engine
- aligned agent execution so CLI and API use the same shared runtime path
- updated `.env` handling to auto-load from the workspace root
- improved provider-aware API key and base URL resolution
- updated `.env.example` to reflect keyless local provider support

### Fixed

- fixed local OpenAI-compatible provider support without requiring a fake API key
- fixed provider-specific API key resolution across multiple configured providers
- fixed Axum route syntax compatibility for current API startup
- fixed `/agents/run` local-provider behavior to match the shared config rules
- fixed pipeline session bookkeeping so API pipeline runs store memory and emit lifecycle events

### Verified

- `cargo build -p lab-cli`
- `cargo test -p lab-api --tests -- --nocapture`
- `cargo test --workspace -- --nocapture`
