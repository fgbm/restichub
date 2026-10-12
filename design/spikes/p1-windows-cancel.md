# Spike P1: cancel restic from a Windows service without a console

Status: complete. Date: 2026-10-12.

Related items: FR-AG-10, AC-AG-06, AC-AG-07. [roadmap.md](../roadmap.md) holds the entry for this spike. [discovery.md](../discovery.md) section 3.5 holds the background.

## Question

The agent runs as a Windows service under the LocalSystem account. A service has no console. Can the agent stop a child `restic.exe` in a graceful way? A graceful stop must let restic remove its lock.

restic is a Go program. On Windows a Go program reacts only to the Ctrl+C and Ctrl+Break console events. A service cannot send these events without a console. This spike finds the mechanism that works.

## Hypotheses

- **A.** The service sends Ctrl+Break to the child process group with `GenerateConsoleCtrlEvent`. The test expects a failure. The call needs a console in the service.
- **B.** The service calls `AllocConsole` before it starts the child. The child shares the new console. Then `GenerateConsoleCtrlEvent` reaches the child. The test expects success.
- **C.** A helper process attaches to the console of the child. The helper sends Ctrl+Break to all processes of that console. The test expects success. The test uses `CREATE_NO_WINDOW` and `CREATE_NEW_CONSOLE` for the child.
- **D.** The service puts the child in a job object and calls `TerminateJobObject`. This mechanism is the hard kill baseline. The lock stays for up to 30 minutes.

## Method

The workflow [spike-p1.yml](../../.github/workflows/spike-p1.yml) runs the experiment on `windows-latest` and on `windows-11-arm`. The workflow checks the SHA-256 checksum of the restic archive. When gpg is available, it also verifies the `SHA256SUMS.asc` signature with the restic release key. The key fingerprint is `CF8F18F2844575973F79D4E191A6868BD3F7A907`, from the restic documentation (`doc/020_installation.rst`).

The program in [spikes/p1-windows-cancel/](../../spikes/p1-windows-cancel/) does these steps for each mechanism:

1. Create a new repository with `restic init`.
2. Start `restic backup --json` on a source of about 2 GB.
3. Wait for the first JSON `status` line or for 5 seconds.
4. Wait for the repository lock.
5. Send the stop of the mechanism.
6. Wait up to 30 seconds for the exit.
7. Count the locks with `restic list locks --no-lock`.
8. Remove the locks with `restic unlock`.

Each mechanism uses its own new repository. A previous snapshot cannot make the next backup finish early.

The program reads the password from the `RESTIC_PASSWORD` environment variable. In service mode the service manager gives this variable to the service from the service `Environment` registry value. The program never puts the password on a command line. Each child goes into a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.

### Validity and verdict

A run is valid only if the repository held a lock before the stop and the child ran at the stop time. The program records these facts for each mechanism:

- `exited_before_stop`: the child did not run when the program sent the stop.
- `saw_summary`: the restic output holds a JSON `summary` message.
- `interrupt_message`: the restic error output holds an interrupt message.
- `locks_after_stop`: the lock count after the stop.

The program computes a `verdict` from these facts:

| Verdict | Condition |
|---|---|
| `graceful_stop` | The child ran at the stop time. It exited in 30 seconds with exit code 130, or with exit code 1 and an interrupt message. `locks_before_stop > 0` and `locks_after_stop == 0`. |
| `stale_lock` | The child exited, but the repository still holds a lock (`locks_after_stop > 0`). |
| `no_stop` | The child still ran 30 seconds after the stop. |
| `invalid_run` | The run does not obey the other conditions. For example, the repository held no lock before the stop. |

The report holds one record for each mechanism. A record holds the mechanism id, the mode, the repository, the Win32 call results, the facts above, the exit code, and the elapsed time. The record also holds the signal that restic used (`exit_code_130` or `exit_code_1_interrupt_message`).

## Results

Workflow run 38256622004 ran on commit d1639f5. The run produced four reports. The workflow ran the program on two runners:

- `windows-latest` (x64).
- `windows-11-arm` (arm64). restic has no arm64 Windows build. The arm64 runner runs the x64 binary under emulation.

Both runners used restic 0.19.1 (go1.26.4, windows/amd64).

The table holds one row for each mechanism, runner and mode. "Locks before/after" is the lock count before the stop and the lock count after the stop. All runs exited within 30 seconds.

| Mechanism | Runner | Mode | Verdict | Exit code | Locks before/after | Elapsed (ms) |
|---|---|---|---|---|---|---|
| A: direct Ctrl+Break | x64 | console | graceful_stop | 130 | 1/0 | 100 |
| A: direct Ctrl+Break | x64 | service | invalid_run | 0 | 1/0 | 7502 |
| A: direct Ctrl+Break | arm | console | graceful_stop | 130 | 1/0 | 501 |
| A: direct Ctrl+Break | arm | service | invalid_run | 0 | 1/0 | 14901 |
| B: AllocConsole, then Ctrl+Break | x64 | console | graceful_stop | 130 | 1/0 | 200 |
| B: AllocConsole, then Ctrl+Break | x64 | service | graceful_stop | 130 | 1/0 | 100 |
| B: AllocConsole, then Ctrl+Break | arm | console | graceful_stop | 130 | 1/0 | 301 |
| B: AllocConsole, then Ctrl+Break | arm | service | graceful_stop | 130 | 1/0 | 200 |
| C1: helper, CREATE_NO_WINDOW | x64 | console | invalid_run | 0 | 1/0 | 8318 |
| C1: helper, CREATE_NO_WINDOW | x64 | service | invalid_run | 0 | 1/0 | 7802 |
| C1: helper, CREATE_NO_WINDOW | arm | console | invalid_run | 0 | 1/0 | 26001 |
| C1: helper, CREATE_NO_WINDOW | arm | service | invalid_run | 0 | 1/0 | 17695 |
| C2: helper, hidden CREATE_NEW_CONSOLE | x64 | console | invalid_run | 0 | 1/0 | 7821 |
| C2: helper, hidden CREATE_NEW_CONSOLE | x64 | service | invalid_run | 0 | 1/0 | 7465 |
| C2: helper, hidden CREATE_NEW_CONSOLE | arm | console | invalid_run | 0 | 1/0 | 19531 |
| C2: helper, hidden CREATE_NEW_CONSOLE | arm | service | invalid_run | 0 | 1/0 | 19027 |
| D: job object hard kill | x64 | console | stale_lock | 1 | 1/1 | 100 |
| D: job object hard kill | x64 | service | stale_lock | 1 | 1/1 | 100 |
| D: job object hard kill | arm | console | stale_lock | 1 | 1/1 | 602 |
| D: job object hard kill | arm | service | stale_lock | 1 | 1/1 | 100 |

Two runs record a failed Win32 call:

- In mechanism B in console mode, `AllocConsole` failed with error 5 (access denied) on both runners. The process already had a console. The mechanism still worked.
- In mechanism A in service mode, `GenerateConsoleCtrlEvent` failed with error 6 (invalid handle) on both runners. The service has no console.

## Findings

Mechanisms C1 and C2 did not stop restic. The stop did not reach restic. restic completed the backup itself and wrote a summary message. The elapsed time is the normal backup time (about 8 to 26 seconds). The verdict `invalid_run` records this fact.

Mechanism A works in console mode and fails in service mode. Hypothesis A is confirmed. The service has no console, and `GenerateConsoleCtrlEvent` fails with error 6.

Mechanism B works in both modes on both runners. restic exits with code 130. restic removes the lock.

## Decision

The agent uses mechanism B on Windows.

At service start, the agent calls `AllocConsole` and hides the console window. The agent starts restic with `CREATE_NEW_PROCESS_GROUP` in a job object with `KILL_ON_JOB_CLOSE`. To cancel a step, the agent sends `CTRL_BREAK_EVENT` to the child process group. After the grace period, the agent calls `TerminateJobObject`. Then the agent reports `possible_stale_lock`.

Mechanism D confirmed this last behavior. A hard kill leaves the lock in the repository. `restic list locks` shows the lock after the stop in all four D runs.

The decision has these limits:

- The result comes from hosted runners only.
- The run used one restic version (0.19.1).
- The console of a service is on a non-interactive desktop. No user sees the window.

## Effect on FR-AG-10

FR-AG-10 requires two stages: a signal first, then a hard kill. Mechanism B gives the signal on Windows. `TerminateJobObject` gives the hard kill. The requirement is feasible as written. The team can implement AC-AG-06 and AC-AG-07 with this design.

On Unix the agent sends SIGINT. On Windows the agent uses mechanism B. The result changes the agent design in this way:

- The agent calls `AllocConsole` in the service and sends `CTRL_BREAK_EVENT` to the child group. The agent needs no extra process. Do not implement mechanisms A, C1, C2 or D as a signal path.
