pactkit marker PACTKIT_MEASUREMENT docs/specs/STORY-20261026412cfc3ce6a9.md
# Test Cases: STORY-20261026412cfc3ce6a9 — Desktop agent management UI

## TC-01: Catalog and installed state are visible (AC1)

- **Given** a catalog with `claude-code` and a discovery entry for the same agent
- **When** the Agents view loads
- **Then** the row shows supported platform/version, `managed` state, and the available versions; a discovery-only tool appears as unsupported and has no install action

## TC-02: Install, update, and uninstall use the daemon job API (AC2)

- **Given** a cataloged agent with an available version
- **When** the user selects Install
- **Then** the app posts to `/v1/agents/{id}/jobs`, shows a running job, and does not call any package manager or external endpoint

## TC-03: Uninstall shows the dry-run paths first (AC3)

- **Given** a managed agent
- **When** the user selects Uninstall
- **Then** the app requests a dry-run, lists the receipted paths, and only submits the uninstall after explicit confirmation

## TC-04: Job progress and history are accurate (AC4)

- **Given** an active job and a hidden window
- **When** the job changes state or the window becomes visible
- **Then** polling stops while hidden, resumes when visible, and the latest state/error is shown exactly once without duplicate submission

## TC-05: Configuration edits respect revisions and validation (AC5)

- **Given** an agent config at revision 3
- **When** the user saves with an outdated revision or a secret-bearing key
- **Then** the editor keeps its contents, shows the 409/validation error inline, and does not display the secret value

## TC-06: Sync status and refresh are visible (AC6)

- **Given** a stale catalog
- **When** the user opens the sync status and selects Refresh catalog
- **Then** the UI shows the last successful sync and reports the refresh result

## TC-07: States and accessibility are complete (AC7)

- **Given** loading, empty, error, offline, in-progress, and disabled states
- **When** each is rendered and navigated by keyboard
- **Then** every state has text (not color alone), focus is trapped/restored in confirmations, and job updates are announced through `aria-live`

## TC-08: Tests cover the UI surface without real installs (AC8)

- **Given** the Story's Storybook/Vitest suite
- **When** it runs with fixtures
- **Then** catalog states, job progress, config conflict, dry-run confirmation, and disabled actions are covered and no real agent is installed
