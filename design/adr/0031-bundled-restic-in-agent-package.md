# ADR-0031: Agent package with a bundled restic

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-AG-11, FR-PR-09, NFR-CMP-03, NFR-SEC-06

## Context

ADR-0011 fixes the restic CLI integration. FR-AG-11 requires the agent package to contain a tested restic version. NFR-CMP-03 fixes the supported restic versions and the compatibility test suite.

A system restic installs an unknown version on each host. The version drifts from the tested version. The agent then uses an untested restic. AC-AG-09 shows the failure mode: an option needs a newer restic and the job fails.

The control plane must not set the restic path. ADR-0004 and ADR-0021 do not let job data start a program. A restic path from the control plane is a step towards that.

The restic project publishes a signed `SHA256SUMS` file with each release.

## Decision

- The agent package always contains the restic binary next to `restichub-agent`.
- CI downloads the official restic release for each operating system and architecture at a pinned version.
- CI verifies the archive against the signed `SHA256SUMS` of the restic project. The restic release signing key fingerprint is pinned in the repository.
- The agent build embeds the expected SHA-256 of the restic binary for its operating system and architecture.
- At start the agent hashes the restic binary next to itself and refuses a different binary.
- The control plane never sets the restic path. The agent always uses the bundled binary.
- The package contains the restic BSD-2-Clause license text and a third-party notice.
- Package formats in the MVP: `tar.gz` for Linux and `zip` for Windows. The `deb`, `rpm` and MSI formats arrive in 1.0 (FR-IN-02).
- An agent self-update and a restic update use one new agent package. The agent reports the agent version and the restic version (FR-AG-11, FR-PR-09).

## Consequences

- The tested restic version travels with the agent. The host install does not change it.
- Each operating system and architecture build embeds one hash. A restic update needs a new agent build.
- An attacker who replaces the restic binary must also patch the embedded hash in the agent. The check raises the cost of a swapped binary.
- The agent package grows by about 25 MB for each platform.
- The restic BSD-2-Clause license is compatible with the Apache-2.0 license of ResticHub (ADR-0014). The third-party notice carries the license text.

## Threats

| Threat | Control in this record |
|---|---|
| T20: a tampered restic binary on a host | The embedded SHA-256 and the start check refuse a different binary. The control plane cannot set the path. |

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Embed restic inside the agent executable | The agent must extract restic to disk for each run. Windows antivirus products give false positives on a self-extracting binary. |
| Use the system restic | The version drifts from the tested version. AC-AG-09 and FR-AG-11 fail. |
| Build restic from source in CI | It adds a Go toolchain and a reproducibility task. A later record can revisit it. |
