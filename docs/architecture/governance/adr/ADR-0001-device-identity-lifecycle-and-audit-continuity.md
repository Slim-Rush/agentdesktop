# ADR-0001-device-identity-lifecycle-and-audit-continuity: Device identity lifecycle and audit continuity

| Field | Value |
|-------|-------|
| ID | ADR-0001-device-identity-lifecycle-and-audit-continuity |
| Status | accepted |
| Date | 2026-10-26 |
| Supersedes | None |
| Superseded-by | None |

## Context

The controller assigns a random UUID v4 on every enrollment and authenticates the
device through a locally generated key pair and a controller-issued X.509 client
certificate. A reinstall, code-signature change, or lost Keychain item makes the
old private key unusable, so the daemon enrolls again and the controller creates a
new device row. Job rows cascade from the old device, job/audit/usage history is
split across device IDs, and there is no per-user device limit. The controller
also has no read path for its audit table.

The product needs a stable audit story for normal upgrades and explicit handling
for re-enrollment, plus a bounded BYOD device footprint.

## Options Considered

- **Option A — Hardware-backed identity**: bind a Secure Enclave/hardware UUID
  attestation to the device. Strongest continuity, but adds attestation
  infrastructure, privacy review, and platform-specific work.
- **Option B — Random identity plus lineage**: keep the UUID + key pair identity,
  pass the previous device ID during re-enrollment, auto-link the new device only
  when the previous record belongs to the same OIDC subject and hostname, retire
  the old record, keep active-only per-user quota, and retain audit/usage with
  lineage.
- **Option C — Hostname-only merge**: deduplicate by hostname. Simple, but
  hostnames are neither unique nor stable, so it can merge unrelated devices and
  corrupt audit history.

## Decision

Adopt Option B.

- Default `devicePolicy.maxDevicesPerUser` is 5 active devices; retired and
  revoked devices do not count.
- A new enrollment may present `previousDeviceId`. The controller auto-links only
  when that device exists, belongs to the same issuer+subject, has the same
  normalized hostname, and is not revoked.
- Linked re-enrollment retires the old device (`supersededBy`, `retiredAt`), records
  the new device as `replacesDeviceId`, and does not consume an extra active slot.
- Admin actions use retire/revoke; normal deletion does not cascade job, telemetry,
  or audit history.
- Audit becomes readable through admin APIs and follows the device lineage.

## Consequences

- Positive: upgrades and recoverable re-enrollments keep an auditable device
  history; BYOD device counts are bounded; old job/usage records remain available.
- Positive: hostname-only merging is explicitly rejected.
- Accepted risk: a user who controls the OIDC subject and knows the previous
  device ID and hostname can auto-link; this is limited to their own devices and is
  audited. Revoked devices are never auto-linked.
- Accepted risk: a hostname change prevents auto-link; the user needs an admin to
  retire the old device or the limit will apply.
- Follow-up: hardware attestation, per-team/plan quotas, and admin override are
  separate Stories.
- Rollout: migrate the controller first; older daemons without
  `previousDeviceId` still enroll but do not get lineage until upgraded.
