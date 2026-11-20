# ADR-0057: restic 0.20.x is the target version

- Status: accepted
- Date: 2026-11-20
- Requirements: FR-AG-11, NFR-CMP-03
- Acceptance: AC-NFR-09
- Supersedes: the target version in ADR-0011

## Context

ADR-0011 sets restic 0.19.x as the target. Restic 0.20.0 fixes two security issues in restore. It prevents writes outside the restore target when a repository contains a malformed tree. It also corrects file ownership when restore cannot find a user or group by name. See the [restic 0.20.0 release notes](https://github.com/restic/restic/releases/tag/v0.20.0).

## Decision

- Use restic 0.20.x as the target version.
- Keep restic 0.18.1 as the minimum supported version.
- Pin restic 0.20.0 in `packaging/restic.env`. CI uses this pin for tests that need a real restic binary. The agent uses this pin as its last download choice under ADR-0038.
- Verify compatibility with restic 0.18.1, 0.19.1 and 0.20.0.

## Consequences

- Operators can update restic with `restichub-agent restic update`.
- Update hosts that run restore first. Check the installed version after the update.
- An older compatible version can still have the restore security issues.

## Alternatives

| Alternative | Why we did not select it |
| --- | --- |
| Keep restic 0.19.x as the target | The CI pin would lack the restore security fixes. |
| Raise the minimum version to 0.20.0 | Hosts with an older supported version would lose compatibility. |
