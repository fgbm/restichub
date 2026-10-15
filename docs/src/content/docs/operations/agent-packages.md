---
title: Agent packages
description: The content of the agent package and the verification chain of the restic that the agent downloads.
---

The agent package does not hold restic. The package and restic are two separate downloads. The agent downloads and verifies restic itself (ADR-0038). Unpack the package in any folder and run `restichub-agent register`. The agent copies itself into its program directory (`/opt/restichub-agent` on Linux, `%ProgramFiles%\ResticHub\Agent` on Windows) and installs the service from there. See [Register an agent](register-an-agent.md) for the commands and the layout.

## Package content

Each package holds these files:

- `restichub-agent` or `restichub-agent.exe`: the agent binary.
- `LICENSE`: the Apache License 2.0 of ResticHub.
- `THIRD_PARTY_NOTICES`: a note that the agent downloads restic, and the fingerprint of the restic release signing key inside the agent.
- `SHA256SUMS`: the SHA-256 of each file above.

The package format is `tar.gz` on Linux and `zip` on Windows. The package name is `restichub-agent-<version>-<os>-<arch>`. The workflow publishes only these packages as artifacts. A build step passes its output to the package step as an intermediate artifact that expires after one day.

## The verification chain of the agent

When the agent downloads restic it runs this chain:

1. It reads the tag of the latest release from GitHub.
2. It downloads the archive, `SHA256SUMS` and `SHA256SUMS.asc` from the release page, over HTTPS with certificate verification.
3. It verifies the OpenPGP signature of `SHA256SUMS` with the release signing key. The key is part of the agent, and only the key with the pinned fingerprint counts.
4. It checks that `SHA256SUMS` lists the archive and that the SHA-256 of the archive equals the listed value.
5. It checks the version: restic 0.18.1 or later, and major version 0.
6. It unpacks the binary and runs it with `version`. The printed version must equal the version of the archive.
7. It stores the binary in the protected directory and writes the version and the SHA-256 into the protected record.

The fingerprint is `CF8F 18F2 8445 7597 3F79 D4E1 91A6 868B D3F7 A907`. The restic installation document names this fingerprint. The file `packaging/restic-release-key.asc` holds the public key. A test checks that the pin inside the agent equals `packaging/restic.env`.

## The restic of the tests

The CI tests that need a real restic use the script `packaging/fetch-restic.sh`. The file `packaging/restic.env` holds the pinned version and the fingerprint. ADR-0011 sets the target version. The pin is restic 0.19.1. The script runs the same chain with `gpg`. It does not touch the agent package.

A weekly workflow (`.github/workflows/restic-latest.yml`) runs the real-restic tests with the latest restic release. A change in the JSON output of restic shows up there before an operator updates.

## Change the pin

1. Change `RESTIC_VERSION` in `packaging/restic.env`.
2. Run `packaging/fetch-restic.sh` for each target.
3. Run the CI.

## The package check

The workflow `.github/workflows/agent-packages.yml` builds each target. One step unpacks each package. It checks that the package holds the agent, `LICENSE`, `THIRD_PARTY_NOTICES` and `SHA256SUMS`, that the hashes match, and that the package holds no restic (AC-IN-01).
