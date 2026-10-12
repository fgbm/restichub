# Spike P1: cancel restic from a Windows service without a console

Status: in progress. Date: 2026-10-12.

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

The cells stay "pending CI run" until the workflow completes.

| Mechanism | Mode | restic exits in time | Exit code | Locks after the stop | Verdict |
|---|---|---|---|---|---|
| A: direct Ctrl+Break | console | pending CI run | pending CI run | pending CI run | pending CI run |
| A: direct Ctrl+Break | service | pending CI run | pending CI run | pending CI run | pending CI run |
| B: AllocConsole, then Ctrl+Break | console | pending CI run | pending CI run | pending CI run | pending CI run |
| B: AllocConsole, then Ctrl+Break | service | pending CI run | pending CI run | pending CI run | pending CI run |
| C1: helper, CREATE_NO_WINDOW | console | pending CI run | pending CI run | pending CI run | pending CI run |
| C1: helper, CREATE_NO_WINDOW | service | pending CI run | pending CI run | pending CI run | pending CI run |
| C2: helper, hidden CREATE_NEW_CONSOLE | console | pending CI run | pending CI run | pending CI run | pending CI run |
| C2: helper, hidden CREATE_NEW_CONSOLE | service | pending CI run | pending CI run | pending CI run | pending CI run |
| D: job object hard kill | console | pending CI run | pending CI run | pending CI run | pending CI run |
| D: job object hard kill | service | pending CI run | pending CI run | pending CI run | pending CI run |

## Decision

Pending the CI run. The decision selects the mechanism for FR-AG-10.

## Effect on FR-AG-10

FR-AG-10 requires two stages: a signal first, then a hard kill. On Unix the agent sends SIGINT. On Windows the agent needs a mechanism from this spike.

The result changes the agent design in these ways:

- If mechanism B works, the agent calls `AllocConsole` in the service and sends `CTRL_BREAK_EVENT` to the child group. The agent needs no extra process.
- If mechanism B fails and mechanism C works, the agent starts a helper process to send the event. The agent package holds the helper.
- If no mechanism works, the agent skips the graceful stage. The agent reports "possible stale lock" after each cancel.
