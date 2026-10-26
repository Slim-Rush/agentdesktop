# Project Rules — agentdesktop

> The law for this repository. When a Spec, test, and implementation disagree,
> this document says which one wins. Amend it only through an explicit change
> request; add dated lessons under `lessons/` instead of rewriting history.

## 1. Hierarchy of truth

1. **Tier 1 — Specs and test cases** (`docs/specs/`, `docs/test_cases/`)
2. **Tier 2 — Tests** (Rust `#[test]`/`#[tokio::test]`, frontend unit and Storybook tests)
3. **Tier 3 — Implementation** (`crates/`, `frontend/`)

On conflict, change the lower tier. When the Spec itself is wrong, fix the Spec
first, then sync tests and code. Never patch code around a known-bad Spec.

## 2. Build, test, lint

| Purpose | Command |
| --- | --- |
| Full test suite (local) | `cargo test --workspace` |
| Full test suite (CI) | `cargo nextest run --workspace` |
| Rust lint | `make lint` (`cargo fmt --all --check`, clippy with `-D warnings`) |
| UI lint + packaging tests | `cd frontend && pnpm check:lint` |
| UI build | `cd frontend && pnpm build` |
| Full build | `make build` |

The UI is built **before** Rust because its assets are embedded in the binary.
A Rust-only build of a package that embeds UI assets is not a valid acceptance
build. Generated schemas (`schema/`) are produced by `cargo xtask schema`
(`make generate-schema`); CI fails if the committed output differs.

## 3. Architecture boundaries

| Crate | Owns | Must not own |
| --- | --- | --- |
| `core` | Shared models, YAML config, serialization, HTTP helpers, telemetry | Daemon lifecycle, controller state |
| `proto` | Generated protobuf/tonic types | Business logic |
| `agent` | Daemon, discovery, provider reconciliation, enrollment, gateway credentials, local API | Controller persistence |
| `controller` | Fleet API, database, OIDC, device CA, gateway JWTs | Workstation filesystem writes |
| `client` | Typed local transport (Unix socket / named pipe) | Tool-specific policy |
| `usage` | OTLP ingestion and usage rollups | Presentation |
| `agentdesktop` | Unified desktop binary entry point | Duplicated daemon/controller logic |
| `xtask` | Schema generation | Runtime behavior |

Cross-crate dependencies point inward toward `core`/`proto`. A new dependency
between peers requires a Spec change, not an ad-hoc import.

## 4. Security invariants

- Never collect or report MCP command arguments, environment variables, HTTP
  headers, or skill bodies. Inventory reports presence and metadata only.
- Provider API keys stay at the gateway. Workstations receive short-lived
  credentials only.
- Device private keys are generated locally and never leave the device; only a
  certificate signing request may be sent to the controller.
- Managed configuration merges into tool-native files and preserves unrelated
  user settings. Destructive rewrites require an explicit, tested migration.
- Files that can contain hostnames, paths, identities, or credentials are
  created owner-only (`secure_fs::ensure_private_dir` and equivalents).
- Never print or persist secrets in source, logs, tests, or evidence.
- The client label asserted for gateway access is policy input, not proof of the
  calling executable. Do not treat it as an authentication boundary.

## 5. Cross-platform contract

Linux, macOS, and Windows are first-class targets. Platform-specific code is
isolated behind `#[cfg(...)]` with a compiling fallback or an explicit
unsupported-path error. A change that only compiles on one OS is incomplete
until CI says otherwise.

## 6. Test and change discipline

- Write or update a failing test before implementation. Tests live next to the
  code they cover; end-to-end coverage lives under `tests/`.
- Run the mapped/changed tests during development; run the full suite at an
  explicit acceptance or release boundary and report any gap.
- Every behavior change updates the relevant Spec or test case. A Story is not
  done while its Spec, tests, and code disagree.
- Dependency versions are declared once in the workspace `[workspace.dependencies]`
  table; crates inherit them. Bump them through the dependency groups, not by
  editing one crate in isolation.
- Public protobuf and YAML schema changes are backwards compatible unless the
  Spec explicitly declares a breaking migration.

## 7. Lessons

`docs/architecture/governance/lessons/` is append-only. Record a dated Lesson
with `pactkit lesson-append` when a defect or review reveals a rule this
document should have stated. Do not rewrite or delete past Lessons.

## 8. Invariants

- All 529+ tests must pass.
- `make lint` and `pnpm check:lint` must pass.
- `make generate-schema` must leave `schema/` unchanged.
- No secret material in the repository or its history.
