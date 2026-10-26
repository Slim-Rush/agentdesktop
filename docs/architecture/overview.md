# Agentdesktop Architecture Baseline

> Reverse-engineered from the repository at `STORY-202610230bbca14eaf4e`.
> This document describes the implemented system, not the Phase 1 target.
> The target is in `graphs/system_design_target.mmd`.

## 1. Purpose and scope

Agentdesktop gives platform teams visibility and control over AI developer tools
on managed workstations. A local daemon discovers tools, applies tool-native
configuration and sandbox policy, inventories MCP servers and skills without
their secrets, and connects each device to a fleet controller. The controller
stores device state, issues short-lived gateway credentials, and serves a
management UI.

Scope of this baseline: system context, containers, crate responsibilities,
external interfaces, end-to-end flows, security invariants, and build/release
constraints. It does not specify new behavior; Phase 1 changes are separate
Stories.

## 2. System context

| Actor / system | Relationship |
|---|---|
| Developer | Uses the desktop app and the installed coding agents on a workstation |
| Platform administrator | Uses the controller web UI; deploys the controller |
| Agent Desktop (Tauri) | Native tray app; shows local daemon status and tool inventory |
| Agent daemon | Runs on the workstation, owns local configuration and credentials |
| Fleet controller | Server-side control plane; device registry, config, usage, JWKS |
| OIDC identity provider | Authenticates users and issues tokens for enrollment and RPCs |
| LLM gateway (agentgateway) | Receives model traffic; emits OTLP access logs |
| GitHub OAuth | Optional GitHub sign-in flow used by managed tools |

The controller never connects inbound to a workstation. The daemon initiates the
gRPC connection over TLS and presents a device certificate plus an OIDC access
token (`crates/agent/src/remote.rs`, `crates/controller/src/service.rs`).

## 3. Containers

| Container | Code | Responsibility |
|---|---|---|
| Desktop app | `crates/agentdesktop`, `frontend/desktop` | Tauri v2 tray app; commands call the local client |
| Local client | `crates/client` | Typed HTTP over Unix socket (Unix) or named pipe (Windows) |
| Agent daemon | `crates/agent` | Discovery, reconciliation, enrollment, LLM proxy, usage receiver |
| Controller | `crates/controller` | Fleet gRPC, admin HTTP, DB, OIDC, device CA, JWT, usage ingest |
| Controller UI | `frontend/controller` | React SPA embedded into the controller binary |
| Shared UI | `frontend/ui` | Theme, primitives, tool and usage components |
| Controller database | SQLite or PostgreSQL | Devices, discoveries, status, telemetry, usage |
| External services | OIDC, LLM gateway, GitHub | Identity, model traffic, optional GitHub OAuth |

The controller serves the built UI from `frontend/controller/dist` through
`rust_embed` (`crates/controller/src/admin.rs:56-58`). The build fails if the UI
is not built first (`crates/controller/build.rs`).

## 4. Crates and dependency direction

| Crate | Owns | Must not own |
|---|---|---|
| `core` | Shared models, YAML config, serde helpers, HTTP helpers, telemetry setup | Daemon lifecycle, controller persistence |
| `proto` | Generated protobuf/tonic types | Business logic |
| `agent` | Daemon, discovery, provider reconciliation, enrollment, gateway credentials, local API, LLM proxy | Controller database |
| `controller` | Fleet API, admin API, database, OIDC, device CA, gateway JWT, usage | Workstation filesystem |
| `client` | Typed local transport | Tool-specific policy |
| `usage` | OTLP ingestion and usage rollups | Presentation |
| `agentdesktop` | Tauri host, tray, CLI entry point | Duplicated daemon/controller logic |
| `xtask` | Schema generation | Runtime behavior |

Allowed direction: `agent`, `controller`, `client`, `usage`, `agentdesktop`
depend on `core` and `proto`; `core` and `proto` do not depend on the others.
Peer-to-peer dependencies require a Spec change.

## 5. External interfaces

### 5.1 Daemon local HTTP API

Socket or named pipe only (`crates/agent/src/api.rs:200-217`):

| Method | Path | Purpose |
|---|---|---|
| GET | `/v1/health` | Local health and controller connection state |
| GET | `/v1/daemon-info` | Version, scope, config path, state dir, proxy/usage info |
| GET | `/v1/config` | Startup configuration |
| GET | `/v1/effective-config` | Applied remote configuration when controller-managed |
| GET | `/v1/remote-config` | Raw persisted remote YAML |
| GET | `/v1/discovery` | Discovered tools, MCP servers, skills, model runtimes |
| GET | `/v1/enrollment` | Enrollment status |
| POST | `/v1/logout` | Leave the managed session |
| POST | `/v1/telemetry` | Forward a telemetry event |
| GET | `/v1/llm-gateway/credential` | Short-lived gateway credential |
| POST | `/v1/llm-gateway/login` | Start gateway sign-in |
| GET | `/v1/usage` | Device usage report |
| GET | `/v1/programs` | Program management state |
| POST | `/v1/programs/choice` | Change the user's program choice |

Authentication boundary: filesystem permissions on the socket (Unix) or the
named-pipe ACL (Windows). There is no network listener.

### 5.2 Controller admin HTTP API

Loopback listener (`crates/controller/src/admin.rs:75-87`):

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/overview` | Fleet summary |
| GET | `/api/v1/devices` | Device list |
| GET | `/api/v1/daemon-config` | Active daemon configuration |
| GET/DELETE | `/api/v1/devices/{device_id}` | Device detail / delete |
| GET | `/api/v1/settings` | Controller settings |
| GET | `/api/v1/usage` | Usage report |
| GET | `/.well-known/jwks.json` | Gateway JWT public keys |

Authentication boundary: loopback only; there is currently no admin
authentication. This is a known Phase 1 gap (see `STORY-202610260c7300b0bf28`
R10), not a guarantee.

### 5.3 Fleet gRPC API

`crates/proto/proto/fleet.proto:5-12`:

| RPC | Purpose |
|---|---|
| `BeginEnrollment` | Start browser or device-code OIDC enrollment |
| `CompleteEnrollment` | Exchange OIDC evidence and a CSR for a device certificate |
| `RenewDeviceCertificate` | Renew an authenticated device certificate |
| `GetLlmGatewayCredential` | Issue a short-lived gateway JWT |
| `GetDeviceUsage` | Return usage for the calling device |
| `Connect` | Bidirectional stream: config down, inventory/status/telemetry up |

Authentication boundary: TLS client certificate (device identity) plus an OIDC
access token whose principal must match the enrolled user
(`crates/controller/src/service.rs:373-417`).

### 5.4 Local LLM proxy

Four fixed route prefixes (`crates/agent/src/llm_proxy.rs:283-327`):
`/vscode-copilot-passthrough`, `/vscode-copilot-capi`, `/vscode-copilot`, and
`/copilot-cli`. The prefix selects the `client_id`; the pairing value is checked
in constant time. The caller executable is not authenticated; the client label
is policy/attribution input, not proof of identity.

### 5.5 Usage ingestion

agentgateway exports OTLP logs to the controller's `UsageIngest` gRPC service or
to a daemon usage receiver (`crates/usage/src/otlp.rs`, `crates/usage/src/lib.rs`).
The receiver validates a bearer token, stores receipts, and rolls usage up per
hour/day.

## 6. End-to-end flows

### 6.1 Enrollment

1. Daemon `remote.rs::run_session` starts OIDC enrollment (`oidc.rs::enroll`).
2. Controller `begin_enrollment` creates a pending enrollment.
3. Daemon completes OIDC and sends a CSR (`complete_enrollment`).
4. Controller `DeviceCertificateIssuer::issue` signs the certificate and
   `Database::enroll_device` stores the principal.
5. Daemon stores the certificate and private key locally and reconnects.

### 6.2 Configuration push and apply

1. Controller watches a YAML file (`daemon_config.rs`) and publishes a revision.
2. Connected daemons receive `ControllerMessage::DaemonConfig` over `Connect`.
3. Daemon `parse_pushed_config` validates UTF-8, YAML, revision, and hash.
4. `Reconciler::apply_with_report` plans each provider, writes user-level files,
   and returns per-program status.
5. Daemon persists `remote-config.yaml` and reports `ConfigStatus`.

### 6.3 Inventory, telemetry, and status

1. `daemon.rs::refresh_inventory` runs discovery on the configured interval.
2. `remote.rs::send_inventory` sends discovered tools, MCP metadata, skills
   front matter, and model runtimes.
3. Controller stores discoveries and program status; the admin UI reads them.
4. Telemetry events (`session.new`, `tool.use`, `tool.use.input`) flow over the
   same stream when configured.

### 6.4 Gateway credential

1. A managed tool calls the credential helper or the local proxy.
2. Daemon `remote.rs::llm_gateway_credential` calls the controller with its
   device certificate and OIDC token.
3. Controller validates the device, `client_id` allowlist, and subscription
   claim, then `GatewayJwtIssuer::issue` returns a short-lived JWT.
4. The proxy forwards model traffic to the gateway; provider API keys stay at
   the gateway.

### 6.5 Usage

1. agentgateway exports OTLP access logs.
2. Controller `UsageIngest` (or daemon usage receiver) validates the token and
   stores a receipt.
3. Hourly rollups are compacted to daily rollups; `/v1/usage` and
   `/api/v1/usage` read them.

## 7. Security invariants and enforcement

| Invariant | Enforcement |
|---|---|
| Inventory never collects MCP arguments, env, headers, or skill bodies | `crates/core/src/model.rs:46-74`; provider discovery parsers |
| Provider API keys stay at the gateway | `llm_proxy.rs`; credential issuance in `service.rs` |
| Device private keys never leave the device | `identity.rs`, `secret_store.rs`; only a CSR is sent |
| Managed files preserve unrelated user settings | Provider reconcile modules and JSON/TOML merge helpers |
| Sensitive local files are owner-only | `secure_fs.rs`, private state directories |
| Device RPCs require certificate + matching OIDC principal | `service.rs:373-417` |
| The client label is not authentication | `llm_proxy.rs` route prefix + pairing; governance rules §4 |
| No secrets in logs, docs, or evidence | `rules.md`; telemetry field limits |

## 8. Build, test, and release constraints

| Purpose | Command |
|---|---|
| Full test suite (local) | `cargo test --workspace` |
| Full test suite (CI) | `cargo nextest run --workspace` |
| Rust lint | `make lint` (`cargo fmt --all --check`, clippy `-D warnings`) |
| UI lint + packaging tests | `cd frontend && pnpm check:lint` |
| Storybook browser tests | `cd frontend && pnpm test:storybook` |
| UI build | `cd frontend && pnpm build` |
| Schema check | `make generate-schema && git diff --exit-code -- schema` |

The UI must be built before Rust because controller and desktop binaries embed
it (`crates/controller/build.rs`; Tauri `frontendDist`). The release workflows
produce a signed/notarized macOS PKG, a signed Windows MSI, and Linux binaries;
the controller ships as a container and Helm chart.

## 9. Cross-platform contract

Linux, macOS, and Windows are first-class targets. Platform-specific code is
isolated behind `#[cfg(...)]` with a compiling fallback. Windows uses a named
pipe and a Windows service; macOS uses a Unix socket and either a system
LaunchDaemon or a user LaunchAgent; Linux uses a Unix socket and the daemon
binary. CI runs the agent tests on Windows and Linux.

## 10. Deployment and tenancy

One controller instance serves one fleet. The database has no `tenant_id`;
OIDC, device CA, gateway JWT keys, and the daemon configuration are single
instances. Phase 1 uses one controller per tenant; true multi-tenancy requires a
schema and authentication change.

## 11. Open questions

- Q1: The controller accepts SQLite or PostgreSQL. The repository does not state
  which backend is the production default; deployment guidance is `unverified`.
- Q2: `pactkit query` reports "no tests found within 3 caller hops" for several
  symbols. That is a graph statement, not proof that no tests exist; the
  test-case index cites actual tests or marks a manual check.

## 12. References

- C4 model: https://c4model.com/ (system context and container levels)
- Fleet contract: `crates/proto/proto/fleet.proto`
- Governance: `docs/architecture/governance/rules.md`
- Current diagram: `docs/architecture/graphs/system_design.mmd`
- Phase 1 target: `docs/architecture/graphs/system_design_target.mmd`
