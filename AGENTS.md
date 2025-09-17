# Repository Guidelines

## Project Structure & Module Organization
- Rust workspace in `crates/`: `goose` (core), `goose-cli` (CLI), `goose-server` (binary: `goosed`), `goose-mcp`, `goose-bench`, `goose-test`, `mcp-core`, `mcp-client`, `mcp-server`.
- UI: `ui/desktop/` (Electron). Scheduler: `temporal-service/` (Go).
- Tests live per-crate under `tests/` (e.g., `crates/goose/tests/`).
- Entry points: `crates/goose-cli/src/main.rs`, `crates/goose-server/src/main.rs`, `ui/desktop/src/main.ts`, `crates/goose/src/agents/agent.rs`.

## Build, Test, and Development Commands
- Setup: `source bin/activate-hermit` (toolchain/env).
- Build: `cargo build` (debug), `cargo build --release`.
- Release + OpenAPI: `just release-binary`.
- Test (workspace): `cargo test`; crate-only: `cargo test -p goose`; single: `cargo test --package goose --test mcp_integration_test`.
- MCP recording: `just record-mcp-tests`.
- UI/OpenAPI: after server changes run `just generate-openapi`; launch desktop with `just run-ui`; UI tests: `cd ui/desktop && npm test`.

## Coding Style & Naming Conventions
- Format: `cargo fmt` (required before commits).
- Lint: `./scripts/clippy-lint.sh`; fix with `cargo clippy --fix` (no new warnings in PRs).
- Errors: use `anyhow::Result`.
- Naming: modules/files `snake_case`; traits/types `UpperCamelCase`.
- Providers implement `Provider` (see `crates/goose/src/providers/base.rs`).

## Testing Guidelines
- Prefer integration tests under `crates/<crate>/tests/`; keep unit tests near code.
- Name tests descriptively; keep tests fast by default.
- Iterate quickly with package filters: `cargo test -p <crate>`.
- Record MCP behavior when needed: `just record-mcp-tests`.

## Commit & Pull Request Guidelines
- Commits follow Conventional Commits: `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`.
- PRs include a clear summary, linked issues, test plan; add screenshots for UI changes.
- Before opening: run `cargo fmt`, `./scripts/clippy-lint.sh`, relevant `cargo test`; regenerate OpenAPI if server changed (`just generate-openapi`).

## Do Not
- Do not edit `ui/desktop/openapi.json` manually — run `just generate-openapi`.
- Do not edit `Cargo.toml` by hand — use `cargo add`.
- Do not skip formatting or lint checks.
