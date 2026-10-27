# Sprint Board

## 📋 Backlog

## 🔄 In Progress

## 🧪 Verifying

## ✅ Done

## 📦 Archived

### [STORY-202610230bbca14eaf4e] Reverse-engineer the Agentdesktop architecture baseline
> Spec: docs/specs/STORY-202610230bbca14eaf4e.md

- [x] Write architecture overview (context, containers, crates, interfaces, flows, security, build)
- [x] Finalize system_design.mmd container diagram
- [x] Write test-case index mapping interfaces and flows to automated or manual checks
- [x] Reconcile Spec evidence and open questions; verify the diff is docs-only

### [STORY-20261026089ea5bea624] Client-managed agent catalog sync and local configuration
> Spec: docs/specs/STORY-20261026089ea5bea624.md

- [x] Add catalog proto messages and capability fields
- [x] Implement controller catalog loader and GetAgentCatalog
- [x] Implement daemon catalog fetch, verification, and cache sync
- [x] Implement local config store and local API endpoints
- [x] Report AgentState to the controller
- [x] Add tests and update the HLD
- [x] Add effective-config resolver with local/managed modes and If-Match Add LocalConfigValidator secret-key denial and redaction
- [x] Fix QA P1: enforce catalog revision ordering and expiry

### [STORY-202610260c7300b0bf28] User-level agent install and uninstall jobs (macOS first)
> Spec: docs/specs/STORY-202610260c7300b0bf28.md

- [x] Add job proto messages and ListPendingJobs
- [x] Add controller job state machine, persistence, and admin API
- [x] Implement daemon job executor and reconnect handling
- [x] Implement manifest verification and macOS user-level installer
- [x] Implement uninstall receipts, dry-run, and detection
- [x] Add diagnostics, audit, tests, and update the HLD
- [x] Add admin authentication and local-origin job entry Implement journaled crash recovery and receipt object verification
- [x] Fix QA P1: identifier validation, catalog expiry, archive size, transitions

### [STORY-20261026412cfc3ce6a9] Desktop agent management UI and job progress
> Spec: docs/specs/STORY-20261026412cfc3ce6a9.md

- [x] Add desktop types and Tauri commands
- [x] Implement Agents view and navigation
- [x] Implement job progress and history
- [x] Implement per-agent config editor with If-Match
- [x] Add sync status and refresh actions
- [x] Add Storybook/Vitest coverage
- [x] Update target architecture diagram

### [STORY-20261026ae47f2938d17] Chat-based failure diagnosis using local coding agents
> Spec: docs/specs/STORY-20261026ae47f2938d17.md

- [x] Implement assistant provider adapter with argv-only runner
- [x] Add chat, context, and action Tauri commands
- [x] Build Assistant view and failure entry points
- [x] Implement playbooks and confirmed actions
- [x] Add audit and automated assistant tests
- [x] Update target architecture diagram

### [STORY-20261026c0268fd1f364] Device identity lifecycle, per-user limits, and audit continuity
> Spec: docs/specs/STORY-20261026c0268fd1f364.md

- [x] Add device lifecycle migration and policy config
- [x] Implement re-enrollment lineage and per-user device limit
- [x] Enforce lifecycle state in device authentication
- [x] Add audit events and read APIs
- [x] Add admin lifecycle and history UI
- [x] Add migration, concurrency, and UI tests
- [x] Update target architecture diagram

### [STORY-2026102757b539fe89bc] Device identity lifecycle QA remediation
> Spec: docs/specs/STORY-2026102757b539fe89bc.md

- [x] Add loopback admin write authorization and CSRF header
- [x] Send admin write header from controller UI
- [x] Bound enrollment previous ID and hostname
- [x] Fix rejection audit attribution and fail-closed link reasons
- [x] Make lifecycle transitions concurrency-safe
- [x] Separate audit load-more error state in UI
- [x] Add daemon device-limit terminal state
- [x] Add remediation tests and stories
- [x] Update target architecture diagram
