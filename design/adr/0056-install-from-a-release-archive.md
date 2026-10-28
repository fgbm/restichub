# ADR-0056: The control plane installs from a release archive with one script

- Status: accepted
- Date: 2026-10-28
- Requirements: NFR-OPS-01, NFR-SEC-06, NFR-SEC-07, NFR-REL-03. Acceptance: AC-NFR-03, AC-NFR-05, AC-NFR-15.
- Related: ADR-0017 (the KEK provider), ADR-0023 (the secret format and the KEK file).

## Context

The installation guide of the release 0.1.0 clones the repository with `git` into `/opt/restichub`. The script `deploy/reference/init-env.sh` then writes the environment file `.env` and the KEK file `secrets/restichub_kek` into that working tree. Only `.gitignore` keeps the two files out of the history.

These facts cause problems:

- `git clean -fdx`, `git stash --all` or a new clone into the same directory deletes the KEK. Without the KEK, the control plane cannot decrypt the secrets in the database (ADR-0017).
- The production host gets the whole repository: the sources, the spikes, the bench and the UI. The stack needs only `deploy/reference` and the file `crates/restichub-db/sql/bootstrap-roles.sql`.
- The guide names a fixed version. The operator verifies the release by hand with tools that the guide does not list: `cosign`, `docker buildx imagetools` and `jq`. A truncated `cosign` download fails with a crash, not with a clear error.
- The upgrade guide changes `RESTICHUB_IMAGE` by hand, with no backup of `.env` and no check of the running image.

NFR-OPS-01 asks for one command that installs the control plane.

## Decision

1. Each release publishes the file `restichub-deploy-<version>.tar.gz`. The archive holds one directory `restichub-deploy-<version>/` with these files: `compose.yaml`, `Caddyfile`, `bootstrap-roles.sql`, `README.md`, `.env.example`, `install.sh` and `IMAGE`. `IMAGE` holds the reference of the control plane image of the release with its digest. The compose file mounts `./bootstrap-roles.sql`, so the stack needs no other file of the repository.
2. The release workflow writes the archive reproducibly: sorted entries, the commit time as the file time, owner and group 0, and `gzip -n`. The archive and `install.sh` are release files. Each one gets a keyless cosign bundle and a line in `SHA256SUMS` (AC-NFR-03). The release notes give the SHA-256 of both files.
3. One script, `install.sh`, installs, upgrades and moves a server that was installed with `git`. The server needs no `git`. The operator runs the script of the release:

   ```sh
   curl -fsSL https://github.com/fgbm/restichub/releases/download/v<version>/install.sh | sudo bash -s -- --domain <domain> --email <email>
   ```

4. The installation has three places:

   | Place | Content | Owner and mode |
   |---|---|---|
   | `/opt/restichub` | The files of the release archive, and the link `.env` to `/etc/restichub/.env`. The script replaces the directory at each upgrade. | root, `0755` |
   | `/etc/restichub/.env` | The environment file with the passwords | root, `0600` |
   | `/etc/restichub/secrets/restichub_kek` | The KEK file (ADR-0023) | uid 10001, `0400` |
   | Docker volumes | The database (`restichub_pgdata`) and the Caddy state | Docker |

   No upgrade and no delete of `/opt/restichub` touches `/etc/restichub`.
5. Compose finds the environment file through the link `/opt/restichub/.env`, so each `docker compose` command in `/opt/restichub` works with no option. The compose file reads the KEK file from `${RESTICHUB_CONFIG_DIR:-/etc/restichub}/secrets/restichub_kek`. The compose project name stays `restichub`, so the volume `restichub_pgdata` keeps the data of an installation from `git`.
6. The script holds the one implementation of the environment file and the KEK. `deploy/reference/init-env.sh` is removed.
7. The script verifies each file that it uses after itself:
   - It downloads `cosign` at a fixed version and compares the SHA-256 with a fixed value. The values are the same as in `.github/workflows/release.yml`. Then it runs `cosign version`.
   - It verifies the bundle of the archive with `cosign verify-blob`. The expected identity is the release workflow at the tag of the version, `https://github.com/fgbm/restichub/.github/workflows/release.yml@refs/tags/v<version>`, with the GitHub OIDC issuer.
   - It reads the image reference from the verified archive and runs `cosign verify` with the same identity. It pulls the image by that digest.
   - After the start it compares the image of the running `cp` container with the verified reference.
   The script has no option that skips a check.
8. The install mode refuses a server with `/etc/restichub/.env`. It creates the KEK, prints the path and the first 8 characters of the SHA-256 of the KEK, and never prints the KEK. The operator must confirm that a copy of the KEK and of `.env` is outside the server. The script asks on the terminal. For a run without a terminal, the operator gives `--kek-saved`. Without a terminal and without the option, the script refuses before it changes the server.
9. `--upgrade` saves a copy of `.env`, the old release files and a database dump into a new directory below `/var/backups/restichub`. It checks the dump with `pg_restore --list`. Then it replaces the release files, sets the new image in `.env` and starts the stack. On a failure, it prints the commands that put back the old release.
10. `--migrate-from-git <directory>` moves a server that was installed with `git`. It changes nothing before the release files and the image are verified. It copies `.env` and the KEK to `/etc/restichub` with the same mode and owner, and compares the SHA-256 of each copy with its source. It makes a database dump as `--upgrade` does. Then it renames the clone to `<directory>.git-backup-<time>`, installs the release files and starts the stack. It never deletes the KEK of the clone. The operator deletes the clone after the copy of the KEK is confirmed.
11. A test seam exists only in the repository copy of `install.sh`, between two marker lines. It lets the CI smoke test use a local file server, a local registry and a test key. The release workflow removes the block and stops when the published script still holds a name of the seam. So the seam cannot weaken a real run.

## Trust chain

| Step | What proves it |
|---|---|
| `install.sh` | The operator verifies the bundle of the script with `cosign verify-blob` and compares the SHA-256 with the release notes. Or the operator trusts the HTTPS connection to the release page of GitHub. |
| `cosign` | The fixed SHA-256 in the verified script |
| The release archive | The keyless signature of the release workflow at the tag |
| The control plane image | The keyless signature of the release workflow at the tag, on the digest from the verified archive |
| The PostgreSQL and Caddy images | The digests in the verified `compose.yaml` |

## Consequences

- A `git` command can no longer delete the KEK. A delete of `/opt/restichub` deletes no secret and no data.
- The production host holds only the files that the stack needs.
- One command installs a server with verified files. The operator needs no `jq` and no `docker buildx`.
- The upgrade has a backup, a check of the dump and a check of the running image. The migrations still run in the `migrate` service with no other step (AC-NFR-15).
- `cosign` is about 130 MB. The script downloads it into a temporary directory at each run and deletes it after the run.
- The published `install.sh` differs from the repository copy: it holds the version, and it has no test seam. The CI smoke test runs the repository copy. Only the `verify` job of the release workflow and a manual run on a clean server test the keyless path.
- An installation from `git` keeps working with its own compose file. It moves to the new layout only with `--migrate-from-git`.

## Accepted risk

`curl … | sudo bash` runs the script before anything verifies it. An attacker who controls the release page or the TLS connection to GitHub can give a changed script, and that script can skip all checks. The checks of the script protect against a changed archive, a changed image and a broken `cosign` download, not against a changed script. The guide gives the alternative: download the script and its bundle, verify both, and then run the script. The owner accepted the risk of the one-command path on 2026-10-28.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Keep the `git` clone and move only the secrets | The server still gets the whole repository, and needs `git`. |
| Resolve the image digest from the tag with `docker buildx imagetools` or the registry API | The tag can move, and the tools need `jq` or `buildx`. The verified archive gives the digest. |
| Pass `--env-file /etc/restichub/.env` to each `docker compose` command | An operator forgets the option, and the command then fails or uses no secrets. |
| Keep `init-env.sh` next to the script | Two implementations of the KEK creation can drift. |
| A package of the operating system (deb, rpm) | It needs a package repository and one build for each distribution. The stack runs in Docker. |
