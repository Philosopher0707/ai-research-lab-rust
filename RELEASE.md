# Release Guide

## Current Target

Next tagged release: `0.2.0`

## Scope of 0.2.0

- CLI presentation cleanup
- `.env` and provider compatibility fixes
- unified pipeline runtime across CLI and API
- unified agent runtime across CLI and API
- modularized CLI and API entrypoints
- release-facing docs and changelog

## Release Checklist

1. Confirm version metadata.
   The workspace version should match the release tag in [Cargo.toml](/Users/philosopher/archived/ai-research-lab-rust/Cargo.toml).
2. Run formatting.
   `cargo fmt --all`
3. Run focused verification.
   `cargo build -p lab-cli`
   `cargo test -p lab-api --tests -- --nocapture`
4. Run full verification.
   `cargo test --workspace -- --nocapture`
5. Review docs.
   Ensure [README.md](/Users/philosopher/archived/ai-research-lab-rust/README.md), [CHANGELOG.md](/Users/philosopher/archived/ai-research-lab-rust/CHANGELOG.md), and [RUNTIME_MAP.md](/Users/philosopher/archived/ai-research-lab-rust/RUNTIME_MAP.md) match the shipped runtime.
6. Tag the release.
   Example: `git tag v0.2.0`

## Recommended Release Notes

Use this short summary:

`0.2.0` makes the Rust lab substantially more shippable by unifying pipeline and agent runtime paths across CLI and API, improving local provider compatibility, and modularizing the previously monolithic entrypoints.

## Post-Release Follow-Up

- continue reducing deeper internal warning-only code paths
- extend integration coverage beyond pipelines to more API endpoints
- decide whether to expose an explicit architecture or flow command in the CLI
