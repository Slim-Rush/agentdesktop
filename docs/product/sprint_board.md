# Sprint Board

## 📋 Backlog

## 🔄 In Progress

## 🧪 Verifying

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

## ✅ Done

## 📦 Archived
