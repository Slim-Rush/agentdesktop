# STORY-202610230bbca14eaf4e: Reverse-engineer the Agentdesktop architecture baseline

| Field | Value |
|-------|-------|
| ID | STORY-202610230bbca14eaf4e |
| Status | Draft |
| Priority | P1 |
| Release | 0.1.1 |

## Background

Agentdesktop is an existing Rust workspace (`crates/*`) with a pnpm/React/Tauri
frontend. PactKit initialization captured the code graph (215 files, 4,026 nodes,
11,275 edges), wrote `docs/architecture/governance/rules.md`, and rendered the
sprint board, but Tier 1 truth is still empty: no Specs, test cases, or
architecture baseline describe the behavior the code already implements.

Without a baseline, later changes would compare against code (Tier 3) instead of
Specs (Tier 1), contradicting the project's truth hierarchy. This Story
reverse-engineers the repository into a docs-only architecture baseline: system
context and containers, crate responsibilities, external interfaces, end-to-end
flows, security invariants, and a test-case index. It changes no executable
behavior; discrepancies found during the work are recorded as open questions.

## Requirements

### R1: System context and containers (MUST)

The baseline MUST document the system context (actors, external services) and
containers (desktop app, daemon, controller, frontends, data stores) at C4
levels 1–2 in `docs/architecture/overview.md`, and the Mermaid container diagram
in `docs/architecture/graphs/system_design.mmd` MUST agree with that text.
`docs/architecture/graphs/system_design_target.mmd` is the Phase 1 target
architecture and is explicitly not part of this baseline.

### R2: Crate responsibilities and dependency direction (MUST)

The baseline MUST map every workspace crate (`core`, `proto`, `agent`,
`controller`, `client`, `usage`, `agentdesktop`, `xtask`) to its owned
responsibilities and state, and MUST state the allowed dependency direction.
The table MUST match the dependencies declared in `Cargo.toml` files.

### R3: External interface inventory (MUST)

The baseline MUST inventory every external interface and its owning file:
the 14 daemon local HTTP routes under `/v1/*`, the 6 controller admin routes
under `/api/v1/*` plus `/.well-known/jwks.json`, the 6 `FleetAgent` gRPC RPCs,
OIDC enrollment, the local LLM proxy routes, and OTLP usage ingestion. Each entry
MUST name its authentication boundary.

### R4: End-to-end flows (MUST)

The baseline MUST document these flows with entry points and call chains:
device enrollment and authentication, configuration push and reconciliation,
inventory/telemetry reporting, LLM gateway credential issuance and proxying, and
usage collection and reporting. Each flow MUST identify the authoritative data
store and the failure/retry boundary.

### R5: Security invariants and enforcement points (MUST)

The baseline MUST record each invariant from
`docs/architecture/governance/rules.md` §4 together with where it is enforced
(file and symbol). The baseline MUST NOT reproduce secret values, tokens, keys,
or MCP command arguments/environment/headers.

### R6: Build, test, and release constraints (MUST)

The baseline MUST document that UI assets are built before Rust, the workspace
test/lint commands, the schema generation check, and the Linux/macOS/Windows
target contract. Commands MUST be copied from `Makefile` and
`.github/workflows/ci.yml`, not invented.

### R7: Test-case index (MUST)

`docs/test_cases/architecture-baseline.md` MUST map each documented interface or
flow to a verifiable check, marking each check as automated (existing test,
command, or CI job) or manual inspection. The Story MUST NOT add executable
tests or production code.

### R8: Documentation-only change boundary (MUST)

The Story MUST change only files under `docs/`. If reverse engineering reveals a
behavioral discrepancy, it MUST be recorded as an open question in the baseline;
it MUST NOT be fixed in this Story.

### R9: Evidence and references (SHOULD)

Each architectural claim SHOULD cite a repository path and symbol or line, and
the baseline SHOULD include a references section naming the C4 model source and
the repository files that govern the system.

## Technical Design

**Approach.** Document C4 levels 1–2 (system context and containers) because the
repository's boundaries are stable and observable. Levels 3–4 (component and
code) belong to future per-subsystem Specs. The existing crate layout is the
container boundary; no new abstraction is introduced.

**Evidence policy.** Every claim cites a repository path (`file:line`) or a
`pactkit query` result. Claims that cannot be verified from the repository are
marked `unverified` rather than asserted. No external API signature is invented.

**Engineering concerns.** Candidate signals were checked against the planned
(docs-only) change:

| Concern | Decision for this Story |
|---------|--------------------------|
| module-design | Document the existing `Provider` trait and crate boundaries; no new module. |
| api-integration | Inventory existing HTTP/gRPC surfaces and their auth boundaries. |
| configuration | Document the daemon/controller YAML layers and secret handling. |
| write-safety | Document merge-preserving managed-file writes and owner-only creation. |
| database | Document controller tables and rollup retention; no schema change. |
| observability | Document telemetry/logging boundaries; no new instrumentation. |
| graceful-shutdown | Document existing shutdown/retry behavior; no code change. |

No concern creates an implementation duty: the Story adds documentation only.

### Lateral Scan Results

- **Operation:** reconcile a managed developer-tool configuration.
  **Existing implementations:** 9 `Provider` implementations under
  `crates/agent/src/provider/*/mod.rs`; 7 are managed programs
  (`MANAGED_PROGRAMS`, `reconcile/mod.rs:655`) and 2 are discovery-only
  (`cursor`, `ollama`). **Assessment:** reuse the existing `Provider` trait and
  each provider's `plan`; no new abstraction is justified.
- **Operation:** authenticate a fleet RPC.
  **Existing implementations:** one shared path,
  `FleetAgentService::authenticate_device` (`service.rs:373`), used by renew,
  gateway credential, usage, and connect. **Assessment:** reuse; the baseline
  documents this as the single authentication boundary.

### Capability Assessment

| Need | Source | Decision |
|------|--------|----------|
| Developer-tool discovery and reconciliation | Project `Provider` trait and 9 implementations | Reuse |
| Fleet transport | Project `FleetAgent` protobuf/tonic service | Reuse |
| Local desktop IPC | Project `agentdesktop-client` (Unix socket / named pipe) | Reuse |
| Controller persistence | Project `Database` on sqlx | Reuse |
| Architecture diagram | Repository Mermaid convention | Reuse |
| Documentation structure | C4 model levels 1–2 (https://c4model.com/) | Enable (reference only) |

No capability is newly implemented; the capability-design rule's
documentation-only exemption applies.

**Best-practice reference.** C4 model, https://c4model.com/ (levels 1–4 and the
dynamic/deployment supporting views), verified 2026-10-23.

## Implementation Inputs

| Path | Purpose |
|------|---------|
| `README.md` | Product capabilities, supported tools, operating modes |
| `crates/proto/proto/fleet.proto` | Fleet gRPC contract and message model |
| `crates/core/src/model.rs` | Discovery, inventory, health, and status model |
| `crates/core/src/config.rs` | Daemon and controller configuration schema |
| `crates/agent/src/api.rs` | Daemon local HTTP API |
| `crates/agent/src/daemon.rs` | Daemon startup, listeners, usage receiver |
| `crates/agent/src/reconcile/mod.rs` | Reconciliation and apply report |
| `crates/agent/src/remote.rs` | Controller session, retries, config push |
| `crates/agent/src/llm_proxy.rs` | Loopback LLM proxy and credential cache |
| `crates/controller/src/service.rs` | Fleet gRPC service and device auth |
| `crates/controller/src/admin.rs` | Controller admin HTTP API |
| `crates/controller/src/database.rs` | Persistence operations |
| `crates/controller/migrations/*.sql` | Controller schema and indexes |
| `crates/controller/src/{oidc,device_ca,gateway_jwt,usage}.rs` | Enrollment, CA, JWT, usage |
| `crates/agentdesktop/src/main.rs` | Tauri commands and tray application |
| `frontend/{controller,desktop,ui}/src` | UI routes, backend calls, shared components |
| `docs/architecture/governance/rules.md` | Security and change invariants |

## Permission Matrix

| Actor | Action | Resource | Decision |
|-------|--------|----------|----------|
| Device (client certificate + OIDC token) | Renew its certificate | Own device certificate | Allow |
| Device (client certificate + OIDC token) | Request a gateway credential for an allowed `client_id` | Gateway credential | Allow |
| Device (client certificate + OIDC token) | Request a gateway credential for an unlisted `client_id` | Gateway credential | Deny (`permission_denied`, `service.rs:180`) |
| Device | Read another device's usage | Usage records | Deny — the device ID is taken from the certificate, never the request |
| Unauthenticated caller | Open the fleet `Connect` stream | Controller stream | Deny — missing device certificate |
| Caller whose OIDC principal differs from the enrolled user | Any authenticated RPC | Enrolled device | Deny (`permission_denied`, `service.rs:411`) |
| Local process with a valid pairing | Use any configured proxy prefix | Local LLM proxy | Allow — the prefix selects `client_id`; the caller executable is not authenticated |
| Calling tool | Assert a client label | Gateway policy | Attribution only — the label is not proof of executable identity |
| Local process able to reach the loopback admin listener | Read or delete device records | Controller database | Allow — loopback is the only boundary; no admin authentication exists today |
| Device user | Opt out of a configured program | Program selection | Allow only when `programSelection.allowUserChoice` is set |

## Trust Boundaries

- Untrusted developer-tool configuration (JSON, YAML, JSON5, frontmatter) →
  provider discovery parsers validate shape and omit secret-bearing fields →
  the `Discovery` model → controller database and UI.
- Local API request → Unix-socket permissions/group or named-pipe ACL
  (`LocalApiAccess`) → daemon handlers → local state.
- Fleet gRPC request → TLS client-certificate device ID → OIDC access-token
  validation → enrolled-principal match → controller database or operation.
- Controller-pushed daemon YAML → UTF-8 and `parse_daemon` validation plus
  revision/hash check → reconciler → managed tool files.
- Managed-tool request to the loopback proxy → fixed route prefix → constant-time
  pairing check → host/path validation → credential cache and upstream gateway.
- OIDC authorization response or device code → state/PKCE/device-code validation
  → controller certificate issuance.
- Gateway OTLP export → usage bearer token → usage ingest → hourly/daily rollups.
- Admin HTTP request → loopback-only admin listener (no authentication today) →
  controller database.

## Acceptance Criteria

### AC1: Context and containers are documented consistently (R1)

- **Given** `docs/architecture/overview.md` and `docs/architecture/graphs/system_design.mmd`
- **When** a reviewer compares the text with the rendered diagram
- **Then** every actor, container, external service, and protocol edge appears in both, with no diagram-only or text-only component

### AC2: Crate map matches the build graph (R2)

- **Given** the baseline's crate responsibility table
- **When** it is compared with the workspace `Cargo.toml` files
- **Then** all eight crates are listed with owned responsibilities, and every allowed dependency direction matches the declared dependencies

### AC3: Interface inventory is complete and authenticated (R3)

- **Given** the interface inventory
- **When** it is cross-checked against `crates/agent/src/api.rs`, `crates/controller/src/admin.rs`, `crates/controller/src/gateway_jwt.rs`, and `crates/proto/proto/fleet.proto`
- **Then** it lists 14 local HTTP routes, 6 admin routes plus the JWKS route, 6 gRPC RPCs, the four LLM proxy routes, and OTLP ingestion, each with its authentication boundary

### AC4: End-to-end flows are traceable (R4)

- **Given** each documented flow
- **When** it is traced from its entry point in the source
- **Then** the entry point, call chain, authoritative data store, and retry/failure boundary are named, and no step relies on an unverified claim

### AC5: Security invariants map to enforcement points (R5)

- **Given** the security section of the baseline
- **When** it is compared with `docs/architecture/governance/rules.md` §4
- **Then** every invariant is mapped to at least one enforcing file and symbol, and no secret value, token, key, MCP argument, environment variable, or header is present

### AC6: Build and release constraints are reproducible (R6)

- **Given** the build/test section
- **When** its commands are compared with `Makefile` and `.github/workflows/ci.yml`
- **Then** every command matches its source, including UI-before-Rust ordering and the `schema/` diff check

### AC7: Test-case index covers every interface and flow (R7)

- **Given** `docs/test_cases/architecture-baseline.md`
- **When** it is audited against the interface and flow inventory
- **Then** every entry has a check classified as automated or manual, and the Story diff contains no executable test or production source file

### AC8: The change stays documentation-only (R8)

- **Given** the completed Story
- **When** `git diff --stat` is inspected
- **Then** all changed paths are under `docs/`, and any discovered discrepancy appears as an open question rather than a code change

### AC9: Claims are evidenced (R9)

- **Given** a sample of architectural claims
- **When** each is spot-checked in the repository
- **Then** it cites a path and symbol/line or a named `pactkit query` result, the references section names the C4 source, and unverified claims are explicitly marked

## Target Call Chain

1. **Desktop/local client.** `frontend/desktop/src/backend.ts` (`invoke`) →
   Tauri commands in `crates/agentdesktop/src/main.rs` → `crates/client/src/lib.rs`
   → `crates/agent/src/api.rs::router` (Unix socket or named pipe).
2. **Enrollment.** `crates/agent/src/remote.rs::run_session` /
   `enroll_with_retry` → `crates/agent/src/oidc.rs::enroll` →
   `crates/controller/src/service.rs::begin_enrollment` /
   `complete_enrollment` → `crates/controller/src/device_ca.rs::issue` →
   `crates/controller/src/database.rs::enroll_device`.
3. **Configuration push and apply.** `crates/controller/src/daemon_config.rs`
   store → `Connect` stream (`service.rs:253`) → daemon
   `crates/agent/src/remote.rs::apply_parsed` → `Reconciler::plan/apply`
   (`reconcile/mod.rs:39`) → nine `Provider` implementations
   (`crates/agent/src/provider/*/mod.rs`).
4. **Inventory and status.** `crates/agent/src/daemon.rs::refresh_inventory`
   → `crates/agent/src/remote.rs::send_inventory` → controller
   `crates/controller/src/service.rs::connect` → `handle_agent_message` →
   `crates/controller/src/database.rs`.
5. **Gateway credential.** Local API
   `crates/agent/src/api.rs::llm_gateway_credential` or
   `crates/agent/src/llm_proxy.rs::forward` → `remote.rs::llm_gateway_credential`
   → controller `service.rs::get_llm_gateway_credential` →
   `crates/controller/src/gateway_jwt.rs::issue`.
6. **Usage.** agentgateway OTLP → `crates/usage/src/lib.rs::UsageIngest` (controller)
   or the daemon usage receiver (`daemon.rs::start_usage`) →
   `crates/usage/src/store.rs` rollups → daemon `/v1/usage`
   (`api.rs:562`) or admin `/api/v1/usage` (`admin.rs:150`).

## Implementation Steps

| Step | File | Action | Dependencies | Risk |
|------|------|--------|-------------|------|
| 1 | `docs/architecture/overview.md` | Write context, container, crate, interface, flow, security, and build sections (R1–R6, R9) | None | Low |
| 2 | `docs/architecture/graphs/system_design.mmd` | Finalize the Mermaid container diagram to match step 1 | 1 | Low |
| 3 | `docs/test_cases/architecture-baseline.md` | Map every interface and flow to an automated or manual check (R7) | 1, 2 | Low |
| 4 | `docs/specs/STORY-202610230bbca14eaf4e.md` | Keep requirements, AC evidence, and open questions consistent with steps 1–3 (R8) | 1–3 | Low |

## Security Scope

| Check | Applicable | Reason |
|-------|------------|--------|
| SEC-1 | No | docs/tests only |
| SEC-2 | No | docs/tests only |
| SEC-3 | No | docs/tests only |
| SEC-4 | No | docs/tests only |
| SEC-5 | No | docs/tests only |
| SEC-6 | No | docs/tests only |
| SEC-7 | No | docs/tests only |
| SEC-8 | No | docs/tests only |

## Dependency Surface

| Field | Value |
|-------|-------|
| Depends on | None |
| Provides | `architecture-baseline-v1` (documentation contract; no code API) |
| Touches | `docs/specs/STORY-202610230bbca14eaf4e.md`, `docs/architecture/overview.md`, `docs/architecture/graphs/system_design.mmd`, `docs/test_cases/architecture-baseline.md` |
| Conflict risk | LOW |

## Open Questions

- Q1: The controller accepts a SQLite or PostgreSQL `database_url`. The
  repository does not state which backend is the production default; the
  baseline must mark deployment guidance `unverified` unless a deployment file
  proves it.
- Q2: `pactkit query` reports "no tests found within 3 caller hops" for several
  symbols. That is a graph-blast-radius statement, not proof that no tests
  exist; the test-case index must cite actual test files or mark a manual check.

## Out of Scope

- Any source or test code change, refactor, or behavior fix.
- Behavior-level Specs for individual subsystems or flows (future Stories).
- Frontend visual, UX, or accessibility review.
- Reading, reproducing, or documenting secret values, tokens, keys, MCP
  arguments, environment variables, or HTTP headers.
- Historical archaeology of past releases, git history, or superseded designs.
- Hand-maintained machine call graphs; `pactkit query` remains the machine
  source for code structure.
