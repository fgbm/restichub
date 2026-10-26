# ADR-0053: A server script runs in a transient systemd unit

- Status: accepted
- Date: 2026-10-26
- Requirements: FR-HK-08, FR-HK-09, FR-HK-10, NFR-SEC-03
- Amends: [ADR-0037](0037-server-scripts-on-trusted-hosts.md), decision 4

## Context

The systemd unit of the agent is hardened for restic. `ProtectSystem=strict` makes the file system read-only outside the restore directories and the repository roots (ADR-0045). `MemoryDenyWriteExecute=true` and `SystemCallFilter=@system-service` block writable and executable memory and the system calls outside the list. `CapabilityBoundingSet` drops the capabilities that restic does not need.

A script step is not restic. It is arbitrary code that the host administrator allows on purpose (ADR-0037). The sandbox of the unit makes such code fail:

1. `gitlab-ctl backup-etc` runs `chpst`, which calls `setgroups`. The unit has no `CAP_SETGID`, so the call fails with `EPERM` and the script exits with code 111.
2. `gitlab-backup create` starts Ruby on Rails with the `ffi` gem, which allocates a writable and executable page. `MemoryDenyWriteExecute=true` refuses it, so the gem does not load.
3. A tool that writes outside the writable roots, for example a database dump into `/var/backups`, fails with `read-only file system`.

The workaround is a drop-in file that weakens the unit, and then another drop-in, and then a third. The sandbox then protects restic less, and every host needs hand-written configuration. That contradicts the trust mode: the mode already says that the control plane may run any command as root on this host. A sandbox inside code that the host trusts adds no security and breaks ordinary tools.

## Decision

1. On Linux, when the local trust mode is on, the agent starts a script step as a transient systemd service. It is not a child of the agent service. The service gets no sandbox from the agent unit. It has full capabilities, no `MemoryDenyWriteExecute`, no system call filter and the whole file system writable.
2. The agent starts it with `systemd-run --quiet --collect --wait --pipe`. The properties are `KillMode=control-group`, `PrivateTmp=yes` and `WorkingDirectory` in the protected script directory. `RuntimeMaxSec` is the backstop of the step timeout plus one minute. The unit name is `restichub-script-<random>.service`.
3. The environment reaches the interpreter only as environment variables. The agent writes it to a protected file next to the script, and the unit starts `/bin/sh -c 'set -a; . "$1"; shift; exec "$@"'` with that file and the fixed interpreter command line. The quoting is exact, so a value keeps quotes, newlines and backslashes. A value with a NUL byte is refused at admission. The agent removes the file and the script directory when the run ends.
4. A cancel and a timeout stay with the agent: the graceful stop is `systemctl kill --signal=SIGINT`, and the forced stop after the grace period is `systemctl kill --signal=SIGKILL`. Both reach every process of the unit, including a process that left the process group. The agent reaps `systemd-run` and reports the exit code of the unit.
5. At start the agent stops a unit whose name starts with `restichub-script-` and that the previous process left behind. Such a unit survives an agent crash until it ends by itself.
6. On a Linux host without systemd, and on Windows and macOS, the script runs as before: a child of the agent. Windows has no sandbox at all, and macOS has no systemd. The agent logs the fallback.
7. The fallback of decision 6 keeps the current behavior, with the sandbox of the unit. A host that needs the tools above must run systemd on Linux.

## Consequences

- The sandbox keeps protecting restic, the restore and the repository roots (T34). It no longer applies to a script, and a script can do what root can do on the host. T33 already accepts this.
- The common tools work without a drop-in: `chpst`, Ruby with `ffi`, a dump into `/var/backups`, `systemctl` for a service.
- A script no longer inherits the environment of the agent. It sees the environment of the service manager (usually `PATH`, `LANG`), the search path that the agent checked and the step parameters. This is a security gain: an ambient secret of the agent does not reach the script.
- The agent needs `systemd-run` and `systemctl` on `PATH`, and systemd 236 or newer for `--collect`.
- A script can outlive the agent process. `RuntimeMaxSec` bounds it, and the next agent start stops it.
- The step log, the output limit and the secret masking stay the same, because the pipes of the transient unit are the pipes of `systemd-run`.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Keep the sandbox and document drop-in files per tool | Every host needs hand-written configuration, and the sandbox loses settings that protect restic. The user must know systemd internals to run a database dump. |
| Remove the sandbox from the agent unit | Restic, the restore and a local repository lose the protection of T34 and ADR-0045. |
| Keep a second installed unit per script | It needs a unit file for each script or a template with a shell wrapper, and the environment still needs a file. The transient unit gives the same with one command. |
| Run the script through a shell that writes the body to `/tmp` | The body and the secrets would leave the protected directory. |
