---
title: Agent packages
description: The content of the agent package and the verification chain of the bundled restic binary.
---

The agent package holds the agent and the restic binary. The agent uses the bundled restic only (ADR-0031). The host install cannot change the tested restic version.

## Package content

Each package holds these files:

- `restichub-agent` or `restichub-agent.exe`: the agent binary.
- `restic` or `restic.exe`: the pinned restic binary.
- `LICENSE`: the Apache License 2.0 of ResticHub.
- `THIRD_PARTY_NOTICES`: the license text of restic, the BSD 2-Clause License.
- `SHA256SUMS`: the SHA-256 of each file above.

The package format is `tar.gz` on Linux and `zip` on Windows. The package name is `restichub-agent-<version>-<os>-<arch>`.

## The pinned version

The file `packaging/restic.env` holds the pinned restic version and the release signing key fingerprint. ADR-0011 sets the target version. The pin is restic 0.19.1.

The fingerprint is `CF8F 18F2 8445 7597 3F79 D4E1 91A6 868B D3F7 A907`. The restic installation document names this fingerprint. The file `packaging/restic-release-key.asc` holds the public key.

## The verification chain

The script `packaging/fetch-restic.sh` runs this chain:

1. It downloads `SHA256SUMS` and `SHA256SUMS.asc` from the restic release page.
2. It imports the public key from `packaging/restic-release-key.asc`. It does not use a keyserver.
3. It verifies the signature of `SHA256SUMS`. It refuses a signature from another key.
4. It downloads the archive for the target. It compares the SHA-256 of the archive with `SHA256SUMS`.
5. It unpacks the binary and prints the SHA-256 of the binary.
6. It downloads the source archive of the release. It verifies the archive and extracts the restic license text.

The agent build reads the printed SHA-256 from the `RESTICHUB_RESTIC_SHA256` variable. The agent embeds this value. At start the agent hashes the bundled restic and refuses a different binary (T20).

The workflow `.github/workflows/agent-packages.yml` runs the chain for each target. One step unpacks each package and checks the content and the restic hash (AC-IN-01).

## Change the pin

1. Change `RESTIC_VERSION` in `packaging/restic.env`.
2. Run `packaging/fetch-restic.sh` for each target.
3. Run the agent package workflow.
