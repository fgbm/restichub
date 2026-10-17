# ADR-0044: Repository roots for local repositories

- Status: accepted
- Date: 2026-10-17
- Requirements: FR-RP-02, FR-PR-01, FR-IN-08, AC-RP-01, AC-IN-03
- Related: [ADR-0035](0035-restore-and-snapshot-browsing.md) (restore roots)

## Context

The systemd unit of the agent sets `ProtectSystem=strict`. This setting makes the file system read-only for the service, except the configuration directory, the cache directory and the restore roots in `ReadWritePaths` (ADR-0035). It does not apply to `/home`, `/root` and `/run/user`: the setting `ProtectHome=` controls these directories, and the unit did not set it (systemd, `protect_system_strict_table` in `src/core/namespace.c`). Thus a local repository below `/home` or `/root` was writable, and a local repository at another path was not. For such a path, restic cannot create or write the repository, and an init fails with `read-only file system`. Also, a compromised restic process could write below `/home` and `/root`, for example the file `/root/.ssh/authorized_keys`.

The administrator can edit the unit, but `install-service` writes the unit again and removes the change. The job cannot open the sandbox, because a compromised control plane could then write anywhere on the host.

## Decision

- The host administrator lists the allowed roots of local repositories in the file `restichub-agent-repository-roots.txt`. The file lies next to the configuration directory, like the restore roots file. On Linux the path is `/etc/restichub-agent-repository-roots.txt`.
- The file has the same format and the same checks as the restore roots file. It holds one absolute path on each line. A line that starts with `#` is a comment. It must be a regular file that root owns and that no other account can change. The agent ignores any other file and logs a warning. The agent ignores a root that another account owns or can write, and a root with such an ancestor. `install-service` refuses such a root.
- There is no default repository root. A host without the file accepts no local repository.
- The unit sets `ProtectHome=read-only`. A backup source below `/home` stays readable. Below `/home`, `/root` and `/run/user`, only a root in `ReadWritePaths` is writable.
- The agent and `install-service` refuse a repository root that is `/` or a system directory, holds one, or lies in one. The system directories are `/etc`, `/usr`, `/bin`, `/sbin`, `/lib`, `/lib32`, `/lib64`, `/libx32`, `/boot`, `/proc`, `/sys`, `/dev` and `/run`. They also refuse a root that holds `/var/lib`, a root in a directory `/var/lib/restichub*`, and a root that overlaps a file of the agent. The agent ignores such a root and logs a warning that names the line. `install-service` stops with a message that names the line.
- `install-service` adds each repository root to `ReadWritePaths`. It creates a missing root with access for root only (mode `0700`) and each missing parent with mode `0755`. The administrator runs `install-service` again after a change of the file.
- The agent checks the location of each local repository of a job when it accepts the job. It uses the rule of the restore roots. It resolves links first. It refuses a path with `.` or `..`, a path outside the roots, and a path that overlaps a file of the agent. It rejects the job with the reason `REPOSITORY_PATH_DENIED`. Restic does not start. The control plane stores the code `repository_path_denied`. The user interface names the file and `install-service`.
- Restic can report `read-only file system` on the path of a local repository. The job error is then `repository_read_only`, with the same hint. This happens when the administrator changed the file and did not run `install-service`.
- The check applies on Linux only. Windows has no such sandbox: the service account can write a local repository at any path, as before. The agent reads no repository roots file on Windows.
- The repository roots file and each repository root are protected paths for a restore (ADR-0035). A restore cannot widen the roots, and it cannot write into a repository, also below the restore root `/`.
- `install-service` refuses a restore root and a repository root when one is the other or holds the other. The restore root `/` stays allowed for an in-place restore; the protected paths keep the repository roots out of it.
- The agent does not refuse a local repository inside another local repository. The guide tells the administrator to avoid it.

### Why a second file

The restore roots and the repository roots give two different rights. A restore root lets the control plane write any file of a snapshot there. A repository root lets restic write the files of a repository there. One shared file would make each repository root a restore target. A stolen Operator token could then restore a snapshot over the files of a repository and destroy the backups. One shared file would also put each restore root into the list of repository paths. Two files keep each right as small as the administrator wants. The rules, the checks and the code of the two files are the same.

## Consequences

- An administrator who uses a local repository on Linux writes the file and runs `install-service` once. The guide "Add a repository" says so.
- A local repository that the Linux service could write before this change needs an entry in the new file. Without it, the agent rejects its jobs. This applies to each local repository below `/home` or `/root`, and to each path that an administrator added to the unit by hand.
- A repository root and each of its existing parents must belong to root. A repository below a directory that another account owns, for example `/home/<user>`, cannot get an entry. The administrator moves it to a directory that root owns, for example `/srv/restichub`. The upgrade guide gives the steps.
- With the restore root `/`, an in-place restore below `/home` or `/root` fails with `read-only file system`. The administrator adds `/home` or `/root` to the restore roots file and runs `install-service`.
- An old control plane shows the reason `rejected` for the new reject reason, because it does not know the value.
- The step result of `test_access` with restic exit code 10 carries the code `repository_missing`. The user interface shows it as "No repository at this location". In an init job it is the expected answer before the init step and shows as information, not as an error.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| One file for the restore roots and the repository roots | A repository root would become a restore target, and a restore could overwrite a repository. |
| Remove `ProtectSystem=strict` | The sandbox protects the system files from a compromised restic process. |
| Keep `ProtectHome` off | A compromised restic process could write below `/home` and `/root`. |
| Allow a repository root below a directory of a user account | That account could replace a directory with a link and make the agent write at another path as root. |
| Let the job or the control plane add a writable path | A compromised control plane could make the whole host writable. |
| A default repository root | A default path that nobody chose fills a disk that nobody watches. The administrator selects the disk. |
