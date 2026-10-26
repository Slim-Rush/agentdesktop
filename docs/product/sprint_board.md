# Sprint Board

## 📋 Backlog

### [STORY-202610260c7300b0bf28] User-level agent install and uninstall jobs (macOS first)
> Spec: docs/specs/STORY-202610260c7300b0bf28.md

- [ ] Add job proto messages and ListPendingJobs
- [ ] Add controller job state machine, persistence, and admin API
- [ ] Implement daemon job executor and reconnect handling
- [ ] Implement manifest verification and macOS user-level installer
- [ ] Implement uninstall receipts, dry-run, and detection
- [ ] Add diagnostics, audit, tests, and update the HLD
- [ ] Add admin authentication and local-origin job entry Implement journaled crash recovery and receipt object verification

## 🔄 In Progress

### [STORY-20261026089ea5bea624] Client-managed agent catalog sync and local configuration
> Spec: docs/specs/STORY-20261026089ea5bea624.md

- [x] Add catalog proto messages and capability fields
- [x] Implement controller catalog loader and GetAgentCatalog
- [ ] Implement daemon catalog fetch, verification, and cache sync
- [x] Implement local config store and local API endpoints
- [ ] Report AgentState to the controller
- [ ] Add tests and update the HLD
- [x] Add effective-config resolver with local/managed modes and If-Match Add LocalConfigValidator secret-key denial and redaction

## 🧪 Verifying

### [STORY-202610230bbca14eaf4e] Reverse-engineer the Agentdesktop architecture baseline
> Spec: docs/specs/STORY-202610230bbca14eaf4e.md

- [x] Write architecture overview (context, containers, crates, interfaces, flows, security, build)
- [x] Finalize system_design.mmd container diagram
- [x] Write test-case index mapping interfaces and flows to automated or manual checks
- [x] Reconcile Spec evidence and open questions; verify the diff is docs-only

## ✅ Done

## 📦 Archived
