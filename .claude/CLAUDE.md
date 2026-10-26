<!-- pactkit:start -->
# agentdesktop — Project Context

## Dev Commands

During development, run changed and mapped tests first. Run the full suite once at an explicit acceptance or release boundary; report any coverage gap.

```bash
# Full acceptance suite (CI uses `cargo nextest run --workspace`)
cargo test --workspace

# Lint: rustfmt + clippy, then UI biome checks
make lint
cd frontend && pnpm check:lint

# Build: UI assets are embedded into the Rust binary, so build the UI first
make build
```

Cold start: run `pactkit context` to load the current project state.

No Python virtualenv is used in this repository.
<!-- pactkit:end -->

@./.claude/CLAUDE.local.md
