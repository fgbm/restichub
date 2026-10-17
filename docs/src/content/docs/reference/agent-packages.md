---
title: Agent packages
description: The content of the agent package and the verification chain of the restic that the agent downloads.
---

The agent package does not hold restic. The package and restic are two separate downloads. The agent downloads and verifies restic itself. Unpack the package in any folder and run `restichub-agent register`. The agent copies itself into its program directory (`/opt/restichub-agent` on Linux, `%ProgramFiles%\ResticHub\Agent` on Windows) and installs the service from there. See [Register an agent](/restichub/getting-started/add-a-host/) for the commands and the layout.

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

The CI tests that need a real restic use the script `packaging/fetch-restic.sh`. The file `packaging/restic.env` holds the pinned version and the fingerprint. The project sets the target version. The pin is restic 0.19.1. The script runs the same chain with `gpg`. It does not touch the agent package.

A weekly workflow (`.github/workflows/restic-latest.yml`) runs the real-restic tests with the latest restic release. A change in the JSON output of restic shows up there before an operator updates.

## Change the pin

1. Change `RESTIC_VERSION` in `packaging/restic.env`.
2. Run `packaging/fetch-restic.sh` for each target.
3. Run the CI.

## The package check

The workflow `.github/workflows/agent-packages.yml` builds each target. One step unpacks each package. It checks that the package holds the agent, `LICENSE`, `THIRD_PARTY_NOTICES` and `SHA256SUMS`, that the hashes match, and that the package holds no restic.

## The restic binary

The agent package does not hold restic. The agent downloads restic itself. `register` and `install-service` download it when the host has none. `run` never downloads. The service makes no network request for restic.

The agent takes the latest release of restic from GitHub. It verifies the OpenPGP signature of the file `SHA256SUMS` with the release signing key of the restic project, which is part of the agent. Then it checks the hash of the archive. It installs only restic 0.18.1 or later, and it refuses a release of a major version above 0. A failed check installs nothing.

The agent stores the binary in a protected directory:

- Linux: `/opt/restichub-agent/bin/restic`.
- Windows: `%ProgramFiles%\ResticHub\Agent\restic.exe`, next to the agent executable. Only SYSTEM and Administrators can write there.

The protected configuration directory holds the file `restic.json` with the version and the SHA-256 of the installed binary. At each start, and before each restic process, the agent checks the binary against this record and refuses a different binary. The control plane cannot set the restic path. If restic is absent or does not match, the service stops with a message that names the next step: run `restichub-agent restic update`.

### Update restic

Run the command as `root` or in an elevated PowerShell:

```sh
restichub-agent restic update
```

The command downloads the latest release and verifies it. It runs the new binary with `version`. Only then does it replace the old binary with one rename. The old binary stays in the same directory as `restic-previous`. If a step fails, the old binary and the old record stay as they were. The service reads the record before each restic process, so the next job uses the new binary. If a service is installed, the command restarts it. Use `--no-restart` to restart it yourself later. On Windows the replacement fails while the service runs the binary. Then stop the service, run the command again and start the service.

The command refuses a release that is older than the installed one. Use `--allow-downgrade` to install it.

The latest release can fail the checks. Examples are a new major version, a changed signature format or a GitHub rate limit. Then the command takes the newest supported release of the last ten GitHub releases. If that also fails, it uses the release that this agent build pins. If no release installs, the message lists each failure. Then use the offline files.

### An offline host

A host without access to GitHub uses files that you copied from another computer. Download these three files from the restic release page of the version that you want:

- `restic_<version>_linux_amd64.bz2` (Linux) or `restic_<version>_windows_amd64.zip` (Windows). Use `arm64` for an ARM host.
- `SHA256SUMS`
- `SHA256SUMS.asc`

Do not rename the archive, because its name gives the version. Then run any of the commands `register`, `install-service` or `restic update` with the options:

```sh
restichub-agent restic update \
  --restic-archive ./restic_0.19.1_linux_amd64.bz2 \
  --restic-sums ./SHA256SUMS \
  --restic-sums-sig ./SHA256SUMS.asc
```

The agent verifies these files in the same way as a download.

### The proxy of the download

The download honours `HTTPS_PROXY` and `NO_PROXY` of the process that runs the command. The proxy must be an `http://` URL, for example `http://proxy.example:3128`. The agent opens a `CONNECT` tunnel through it, and the TLS session and all checks stay between the agent and GitHub. A proxy with a user name and a password uses `http://user:password@proxy.example:3128`. Write a special character of the user name or the password as a percent escape, for example `%40` for `@`. An IPv6 proxy address uses brackets, for example `http://[2001:db8::1]:3128`.

`sudo` removes these variables. Keep them with `sudo --preserve-env=HTTPS_PROXY,NO_PROXY restichub-agent restic update`.

GitHub limits the number of requests without a token. If GitHub answers with a rate limit, the agent says so. Try again later or use the offline files.
