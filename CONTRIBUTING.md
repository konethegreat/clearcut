# Contributing

Thanks for helping. ClearCut is a multi-language monorepo; see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Setup
- Rust (stable) for `crates/`, Node 22 for `packages/`.
- `cargo test --workspace` and `npm ci && npm run build && npm test`.
- The `clearcut-store` tests need a Postgres (13+) and a `DATABASE_URL`; each test gets its own throwaway database. Quick local setup (dev-only password, a spare port so it cannot clash with a Postgres you already run):
  ```
  docker run -d --name clearcut-pg -e POSTGRES_USER=clearcut -e POSTGRES_PASSWORD=clearcut_dev -e POSTGRES_DB=clearcut -p 127.0.0.1:54329:5432 postgres:17
  export DATABASE_URL=postgres://clearcut:clearcut_dev@127.0.0.1:54329/clearcut
  ```
  (PowerShell: `$env:DATABASE_URL = "postgres://clearcut:clearcut_dev@127.0.0.1:54329/clearcut"`.) CI runs the same tests against a Postgres service container.

## Ground rules
1. **Money is integer minor units.** Never use floats for amounts.
2. Business rules live in `clearcut-core` only; transports (REST, MCP) stay thin.
3. Every rule needs a test, and compliance rules cite their source in a comment.
4. Run `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` before pushing.
5. Use conventional commits (`feat:`, `fix:`, `docs:`, `chore:`).
6. Record notable work in [docs/WORKLOG.md](docs/WORKLOG.md); decisions go in `docs/adr/`.

By contributing you agree your work is licensed under Apache-2.0.
