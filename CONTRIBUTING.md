# Contributing

Thanks for helping. ClearCut is a multi-language monorepo; see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Setup
- Rust (stable) for `crates/`, Node 22 for `packages/`.
- `cargo test --workspace` and `npm ci && npm run build && npm test`.

## Ground rules
1. **Money is integer minor units.** Never use floats for amounts.
2. Business rules live in `clearcut-core` only; transports (REST, MCP) stay thin.
3. Every rule needs a test, and compliance rules cite their source in a comment.
4. Run `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` before pushing.
5. Use conventional commits (`feat:`, `fix:`, `docs:`, `chore:`).
6. Record notable work in [docs/WORKLOG.md](docs/WORKLOG.md); decisions go in `docs/adr/`.

By contributing you agree your work is licensed under Apache-2.0.
