# ADR-0038: The agent downloads restic

- Status: accepted
- Date: 2026-10-15
- Requirements: FR-AG-11, FR-IN-07, FR-IN-08, NFR-CMP-03, NFR-SEC-06
- Supersedes: the bundling part of ADR-0031

## Context

ADR-0031 puts a pinned restic binary into the agent package. The agent build embeds the SHA-256 of that binary. The owner decided on 2026-10-15 that the agent package must not contain restic. The agent package and restic are two separate downloads. The agent downloads restic itself.

The preferred source is the portal of ResticHub, which serves restic. That source does not exist yet. For the MVP the source is the latest release of restic on GitHub.

A download must keep the integrity model of ADR-0031. The restic project signs the `SHA256SUMS` file of each release. The control plane must not set the restic path (FR-AG-11).

## Decision

### Source

- The source of restic is a value of the `Source` type in the agent. The MVP has one value: the release assets of `restic/restic` on GitHub. A later record adds the portal of ResticHub as a second value.
- The agent asks `https://api.github.com/repos/restic/restic/releases/latest` for the tag. It then downloads `restic_<version>_<os>_<arch>.bz2` (Linux) or `.zip` (Windows), `SHA256SUMS` and `SHA256SUMS.asc` from `https://github.com/restic/restic/releases/download/v<version>/`.
- The agent uses HTTPS only and always verifies the server certificate. It follows at most five redirects and refuses a redirect to a URL that is not `https://`. Each file has a size limit, and each network step has a time limit.
- The agent honours `HTTPS_PROXY` (an `http://` proxy URL, with a `CONNECT` tunnel) and `NO_PROXY`.
- A rate limit of the GitHub API gives a clear error that names the offline option.
- An offline host uses `--restic-archive`, `--restic-sums` and `--restic-sums-sig`. The agent verifies these files in the same way as a download.

### Integrity

- The agent embeds the public restic release signing key from `packaging/restic-release-key.asc` and the pinned fingerprint `CF8F18F2844575973F79D4E191A6868BD3F7A907`. The agent takes the key with this fingerprint from the key block and ignores other keys.
- The agent verifies the OpenPGP signature `SHA256SUMS.asc` over `SHA256SUMS`. Then it checks that `SHA256SUMS` lists the archive once, and that the SHA-256 of the archive equals the listed value. Only then does it unpack the archive.
- The unpacked binary must run `restic version` and must print the version of the archive.
- The OpenPGP check is a small module of the agent. It accepts only a version 4 RSA key and a version 4 signature of a binary document. The hash is SHA-256, SHA-384 or SHA-512. The issuer fingerprint must equal the pinned one. It rejects everything else. It uses `ring` for the RSA check, which the TLS stack already uses, and `sha1` for the key fingerprint. A general OpenPGP library (`pgp` and its dependencies) adds about 100 crates to a program that runs as root. A test verifies the real signature of restic 0.19.1 with the module.
- Unpacking uses the `bzip2` crate (pure Rust `libbz2-rs-sys`) and the `zip` crate with the `deflate` decoder. Both have size limits. `deny.toml` allows the `bzip2-1.0.6` license of `libbz2-rs-sys`.

### Supported versions

- The minimum version is restic 0.18.1 (NFR-CMP-03).
- The tested major version is 0. The agent refuses a release of a higher major version, because the compatibility suite does not cover it. The same checks run at each start for the recorded version.

### Location and record

- The binary lives in a protected directory: `/opt/restichub-agent/bin` on Linux and `%ProgramFiles%\ResticHub\Agent` on Windows (see the notes of 2026-10-15). Only root or Administrators can write there.
- The program directory and the data directory are in the protected set of the restore access (FR-RS-03). A restore never writes there.
- After an install the agent writes a record `restic.json` in the protected configuration directory. The record holds the version and the SHA-256 of the installed binary.
- At every start the agent reads the record and checks that the binary hash equals the recorded hash. The agent checks the hash again before each spawn (T20). A missing record, a missing binary or a different hash stops the start with a message that names `restichub-agent restic update`.
- The agent no longer embeds a restic hash at build time. The build script and the `dev-unpinned-restic` feature are gone.

### When the agent downloads

- `register` installs restic when no record exists. It does this after the preflight and before the enrollment request. A host without internet fails before the one-time token is spent.
- `install-service` installs restic when no record exists.
- `restichub-agent restic update` needs elevation. It fetches the latest release, verifies it, runs it as `restic version`, and only then replaces the installed binary with one rename. The previous binary stays as `restic-previous`. Any failure leaves the old binary and the old record in place. The service reads the record before each restic process, and the command restarts an installed service unless `--no-restart` is given. If the latest release fails the checks, the command takes the newest supported release of one bounded page of the release list. Then it takes the release pinned in `packaging/restic.env`. The command refuses a release older than the installed one unless `--allow-downgrade` is given.
- `run` never downloads. The service makes no network request for restic. If restic is absent, it refuses to start with a clear message.

### Package

- The agent package holds `restichub-agent`, `LICENSE`, `THIRD_PARTY_NOTICES` and `SHA256SUMS`. It does not hold restic.
- `packaging/fetch-restic.sh` and `packaging/restic.env` stay. CI uses them to get a pinned restic for the tests. A weekly CI job runs the real-restic tests with the latest restic release.

## Consequences

- The agent package is about 25 MB smaller. The host needs access to GitHub or the offline files once.
- The restic version is not tied to the agent build any more. The operator chooses when to update. The compatibility tests and the minimum version limit the risk.
- The signed `SHA256SUMS` protects the download. An attacker needs the private key of the restic project to forge a release. A release that restic signed but that is older than the latest one can be replayed only inside the supported version range.
- An attacker with write access to the binary must also change the protected record. Both need administrator rights. This is the same level of protection as the embedded hash gave. T20 stays accepted for an attacker with those rights.
- A crash between the binary swap and the record write leaves a mismatch. The agent then refuses to start until the operator runs `restichub-agent restic update`.
- The agent trusts the root certificates of the host to reach GitHub. The signature check does not depend on this trust.

## Threats

| Threat | Control in this record |
|---|---|
| T20: a tampered restic binary on a host | The signed `SHA256SUMS`, the archive hash, the protected directory, and the protected record checked at each start and spawn. The control plane cannot set the path. |
| T20: a network attacker serves a forged restic | Server certificate verification, the OpenPGP signature of `SHA256SUMS` with the pinned key, and the archive hash. |
| T20: a downgrade to a vulnerable or untested restic | The minimum version and the tested major version. |

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Keep restic in the agent package (ADR-0031) | The owner wants two separate downloads. |
| Use the system restic | The version drifts from the tested version (ADR-0031). |
| Use a general OpenPGP library | About 100 more crates in a program that runs as root, for one signature algorithm. |
| Pin the hash of one restic version in the agent | A restic update then needs a new agent build. |
| Wait for the portal | The portal does not exist yet. `Source` leaves room for it. |

## Note of 2026-10-15: program directory

The owner decided on 2026-10-15 to keep executables apart from data.

- Executables live in the program directory. Windows uses `%ProgramFiles%\ResticHub\Agent\restichub-agent.exe` and `%ProgramFiles%\ResticHub\Agent\bin\restic.exe`. Linux uses `/opt/restichub-agent/bin/restichub-agent` and `/opt/restichub-agent/bin/restic`.
- Data lives in the data directories only: `%ProgramData%\ResticHub\Agent` on Windows, and `/etc/restichub-agent` and `/var/lib/restichub-agent` on Linux. The identity, the journal, the cache and the record `restic.json` stay there.
- The earlier locations `%ProgramData%\ResticHub\Agent\bin` and `/var/lib/restichub-agent/bin` are legacy. `register` and `install-service` move restic from there into the program directory, remove the old copies and keep the identity and the record.
- The agent copies itself into the program directory. Only SYSTEM and Administrators can write there (Linux: root-owned, mode 0755). The service starts from there. This keeps the location check of threat T20 true by construction.

## Note of 2026-10-15: restic next to the agent on Windows

The owner decided on 2026-10-15 that restic lives next to the agent executable on Windows.

- Windows uses `%ProgramFiles%\ResticHub\Agent\restic.exe`. There is no `bin` subdirectory. This replaces the Windows path of the note "program directory". Linux does not change.
- `%ProgramFiles%\ResticHub\Agent\bin\restic.exe` and `%ProgramData%\ResticHub\Agent\bin\restic.exe` are legacy. `register`, `install-service` and the start of the service move restic from there to the new path, remove the empty `bin` directory and keep the record. The agent checks that the copy equals the original. This guards the copy only; trust comes from the record. The agent keeps the file whose SHA-256 equals the record and deletes the other files. If that file is an old one, the agent moves it. Without a record, it keeps the new file. The agent does not follow a link or a reparse point. It deletes the old `restic-previous` file. A failed move at the start of the service gives a warning. The hash check stops the start when no valid restic is in place. The hash checks at the start and before each spawn then run against the new path. The record does not hold the path, so no re-register is needed.
