# Architecture Baseline — Test Case Index

> Companion to `docs/architecture/overview.md` and
> `STORY-202610230bbca14eaf4e`. Each row names a verifiable check and marks it
> `automated` (a command/CI job exists) or `manual` (inspection required).
> No executable tests are added by this Story.

## How to run the automated checks

```bash
cargo test --workspace
cargo nextest run --workspace          # CI
make lint
cd frontend && pnpm check:lint
cd frontend && pnpm test:storybook
make generate-schema && git diff --exit-code -- schema
```

## Interfaces

| Area | Check | Type | Evidence |
|---|---|---|---|
| Local API | 14 routes are registered on the socket router | manual | `crates/agent/src/api.rs:200-217` |
| Local API | Program/usage/config helpers behave as documented | automated | `cargo test -p agentdesktop-agent api::tests` |
| Local API | No TCP listener is added for the local API | manual | `crates/agent/src/daemon.rs:1108-1175` |
| Admin API | 6 routes plus JWKS are registered | manual | `crates/controller/src/admin.rs:75-87` |
| Admin API | Admin listener is loopback by default | automated | `cargo test -p agentdesktop-core config::tests` (defaults) |
| Admin API | No admin authentication exists today | manual | `crates/controller/src/admin.rs` (no auth middleware) |
| Fleet gRPC | 6 RPCs are declared | manual | `crates/proto/proto/fleet.proto:5-12` |
| Fleet gRPC | Device certificate + OIDC principal binding | automated | `cargo test -p agentdesktop-controller service::tests::device_authentication_binds_oidc_issuer_and_subject` |
| Fleet gRPC | Legacy device without issuer fails closed | automated | `cargo test -p agentdesktop-controller service::tests::legacy_device_without_enrolled_issuer_fails_closed` |
| LLM proxy | 4 route prefixes and credential modes | manual | `crates/agent/src/llm_proxy.rs:264-327` |
| LLM proxy | Route matching, pairing, and forwarding | automated | `cargo test -p agentdesktop-agent llm_proxy` |
| Usage | OTLP logs ingestion and token check | automated | `cargo test -p agentdesktop-usage` |
| JWKS | Public keys are served with cache headers | manual | `crates/controller/src/gateway_jwt.rs:38-56` |

## Flows

| Flow | Check | Type | Evidence |
|---|---|---|---|
| Enrollment | Begin/complete/CSR/certificate chain is traceable | manual | `overview.md` §6.1; `service.rs:82-144`; `device_ca.rs` |
| Enrollment | OIDC principal must match the enrolled user | automated | `cargo test -p agentdesktop-controller service::tests` |
| Config push | Revision/hash parsing and apply path | manual | `overview.md` §6.2; `remote.rs:889-945` |
| Config push | Reconciler applies provider plans | automated | `cargo test -p agentdesktop-agent reconcile::tests` |
| Inventory | Discovery and inventory stream are traceable | manual | `overview.md` §6.3; `daemon.rs:876-940`; `remote.rs:1052-1115` |
| Inventory | Discovery model omits secret fields | automated | `cargo test -p agentdesktop-core model::tests` (where present) |
| Gateway credential | Device + client allowlist + subscription claim | automated | `cargo test -p agentdesktop-controller service::tests::subscription_billing_requires_server_configured_program_auth` |
| Gateway credential | Proxy credential selection and forwarding | automated | `cargo test -p agentdesktop-agent llm_proxy` |
| Usage | OTLP receipt, rollup, and report | automated | `cargo test -p agentdesktop-usage` |
| Usage | Admin/device usage endpoints read rollups | manual | `overview.md` §6.5; `api.rs:562-624`; `admin.rs:150-166` |

## Security invariants

| Invariant | Check | Type | Evidence |
|---|---|---|---|
| Inventory omits MCP args/env/headers and skill bodies | manual | `crates/core/src/model.rs:46-74`; discovery parsers |
| Device private key never leaves the device | manual | `identity.rs`; `secret_store.rs`; enrollment sends only a CSR |
| Managed files preserve unrelated user settings | automated | `cargo test -p agentdesktop-agent reconcile::tests` |
| Sensitive local state is owner-only | manual | `secure_fs.rs`; state-directory creation |
| No secrets in docs/evidence | manual | repository review; governance rules §4 |
| Client label is not authentication | manual | `llm_proxy.rs:264-327`; `rules.md` §4 |

## Build, release, and platform

| Area | Check | Type | Evidence |
|---|---|---|---|
| Rust tests | Full suite passes | automated | `cargo test --workspace`; CI `cargo nextest run --workspace` |
| Rust lint | fmt + clippy with `-D warnings` | automated | `make lint`; CI |
| UI lint/packaging | Biome + typecheck + packaging tests | automated | `pnpm check:lint`; CI |
| UI behavior | Storybook browser tests | automated | `pnpm test:storybook`; CI |
| Schema | Generated schema is current | automated | `make generate-schema && git diff --exit-code -- schema` |
| Windows | Agent tests run on Windows | automated | `.github/workflows/ci.yml` `windows` job |
| Linux | Controller image builds | automated | `.github/workflows/ci.yml` `container` job |
| macOS | Signed/notarized PKG is produced | automated | `.github/workflows/release-macos.yml` |
| Windows | Signed MSI is produced and smoke-tested | automated | `.github/workflows/release-windows.yml` |
| Cross-platform code | Platform branches compile | automated | CI matrix; `#[cfg(...)]` review |

## Deployment and tenancy

| Area | Check | Type | Evidence |
|---|---|---|---|
| Tenancy | One controller serves one fleet; no `tenant_id` | manual | `migrations/0001_controller.sql`; `ControllerConfig` |
| Config scope | One global `DaemonConfig` per controller | manual | `daemon_config.rs`; `service.rs:253-330` |
| Admin exposure | Helm service exposes only the fleet port | manual | `deploy/helm/.../service.yaml` |
| Release | Controller image + Helm chart | automated | `release-linux.yml`; `deploy/helm` |

## Manual review checklist

- [ ] Re-run `cargo test --workspace` and `make lint` before acceptance.
- [ ] Confirm no secret values were copied into this baseline.
- [ ] Re-check the admin-authentication gap before exposing the admin port.
- [ ] Re-check the LLM proxy caller-identity statement against any future change.
