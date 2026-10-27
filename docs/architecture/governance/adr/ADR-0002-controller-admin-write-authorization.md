# ADR-0002-controller-admin-write-authorization: Controller admin write authorization

| Field | Value |
|-------|-------|
| ID | ADR-0002-controller-admin-write-authorization |
| Status | accepted |
| Date | 2026-10-27 |
| Supersedes | None |
| Superseded-by | None |

## Context

The controller admin listener is loopback-only. Read endpoints use
`require_if_configured` and work without a token when `adminAuth` is absent.
Mutating endpoints use `authenticate`, which always requires an OIDC bearer
token and a non-empty `allowedSubjects`. The controller UI is served by the same
listener and has no login or token entry, so its retire/revoke actions fail with
401/403 in every configuration. At the same time, lifecycle writes must not be
open to cross-site requests on a developer workstation.

## Options Considered

- **Option A — Loopback writes plus a custom CSRF header**: when `adminAuth` is
  configured, keep bearer-only writes; otherwise accept loopback writes that
  carry `x-agentdesktop-admin: 1`. A form cannot set the header, and cross-origin
  JavaScript with a custom header is preflighted, so the local UI works without
  storing a credential. OWASP documents custom request headers as a CSRF
  defense for AJAX/API endpoints.
- **Option B — OIDC login and httpOnly session for the controller UI**: complete
  SSO, but adds login, redirect, session, logout, and CSRF-token infrastructure.
- **Option C — Startup write token**: the controller prints a random token and
  the UI asks for it. Introduces a second authentication mechanism.
- **Option D — API-only lifecycle**: remove the UI actions. Does not satisfy the
  original Story's admin-UI requirement.

## Decision

Adopt Option A.

- `AdminAuth::authorize_write` delegates to bearer authentication when
  `adminAuth` is configured and non-empty.
- Otherwise it requires the loopback-only CSRF marker header and records the
  actor as `local#loopback-admin`.
- The admin listener validates the HTTP `Host` header against a loopback
  allowlist (`127.0.0.1`, `localhost`, `[::1]`, with the bound port). A
  DNS-rebinding request carries the attacker's host name even though the TCP
  connection reaches 127.0.0.1, so it is rejected before routing.
- `create_job`, retire, and revoke use the same policy; GET endpoints remain
  `require_if_configured`.
- The marker is not a credential and is never stored in browser storage.

## Consequences

- Positive: the local controller UI works without introducing a login flow or
  a second credential, and cross-site form/fetch attempts are blocked by the
  missing header or CORS preflight.
- Positive: the Host allowlist closes the DNS-rebinding path that would
  otherwise let a public page reach the loopback listener under its own host
  name.
- Positive: configured deployments keep fail-closed bearer authorization.
- Accepted risk: only loopback host names are supported; an operator using a
  custom hostname mapped to 127.0.0.1 must access the UI through
  `127.0.0.1` or `localhost`.
- Accepted risk: an `adminAuth`-configured deployment that exposes the UI
  directly still needs an authenticating proxy to inject a bearer token. Admin
  SSO remains a future Story.
- Accepted risk: the header is a CSRF defense, not a network boundary; the
  loopback-only listener remains the primary boundary.
- Follow-up: monitor an authenticated admin UI Story and keep the header
  constant shared between backend and frontend tests.
