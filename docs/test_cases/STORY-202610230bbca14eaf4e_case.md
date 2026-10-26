pactkit marker PACTKIT_MEASUREMENT docs/specs/STORY-202610230bbca14eaf4e.md
# Test Cases: STORY-202610230bbca14eaf4e — Architecture baseline

## TC-01: Context and containers are documented consistently (AC1)

- **Given** `docs/architecture/overview.md` and `docs/architecture/graphs/system_design.mmd`
- **When** a reviewer compares the text with the rendered diagram
- **Then** every actor, container, external service, and protocol edge appears in both, with no diagram-only or text-only component

## TC-02: Crate map matches the build graph (AC2)

- **Given** the baseline's crate responsibility table
- **When** it is compared with the workspace `Cargo.toml` files
- **Then** all eight crates are listed with owned responsibilities, and every allowed dependency direction matches the declared dependencies

## TC-03: Interface inventory is complete and authenticated (AC3)

- **Given** the interface inventory
- **When** it is cross-checked against `crates/agent/src/api.rs`, `crates/controller/src/admin.rs`, `crates/controller/src/gateway_jwt.rs`, and `crates/proto/proto/fleet.proto`
- **Then** it lists 14 local HTTP routes, 6 admin routes plus the JWKS route, 6 gRPC RPCs, the four LLM proxy routes, and OTLP ingestion, each with its authentication boundary

## TC-04: End-to-end flows are traceable (AC4)

- **Given** each documented flow
- **When** it is traced from its entry point in the source
- **Then** the entry point, call chain, authoritative data store, and retry/failure boundary are named, and no step relies on an unverified claim

## TC-05: Security invariants map to enforcement points (AC5)

- **Given** the security section of the baseline
- **When** it is compared with `docs/architecture/governance/rules.md` §4
- **Then** every invariant is mapped to at least one enforcing file and symbol, and no secret value, token, key, MCP argument, environment variable, or header is present

## TC-06: Build and release constraints are reproducible (AC6)

- **Given** the build/test section
- **When** its commands are compared with `Makefile` and `.github/workflows/ci.yml`
- **Then** every command matches its source, including UI-before-Rust ordering and the `schema/` diff check

## TC-07: Test-case index covers every interface and flow (AC7)

- **Given** `docs/test_cases/architecture-baseline.md`
- **When** it is audited against the interface and flow inventory
- **Then** every entry has a check classified as automated or manual, and the Story diff contains no executable test or production source file

## TC-08: The change stays documentation-only (AC8)

- **Given** the completed Story
- **When** `git diff --stat` is inspected
- **Then** all changed paths are under `docs/`, and any discovered discrepancy appears as an open question rather than a code change

## TC-09: Claims are evidenced (AC9)

- **Given** a sample of architectural claims
- **When** each is spot-checked in the repository
- **Then** it cites a path and symbol/line or a named `pactkit query` result, the references section names the C4 source, and unverified claims are explicitly marked
