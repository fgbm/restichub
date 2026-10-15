# restichub-fake-restic

The `restichub-fake-restic` binary emulates the restic command line for the tests of the agent. The binary writes recorded restic JSON output and exits with a selected code. The tests of the agent run it on Linux and on Windows.

The recorded output follows restic 0.19.1. The message shapes come from the restic source at tag `v0.19.1`.

## Commands

| Command | JSON output |
|---|---|
| `backup` | The `status` messages and the final `summary` message |
| `restore` | The `status` messages and the final `summary` message |
| `snapshots` | The array of snapshots |
| `ls` | The `snapshot` message and one `node` message for each entry of the listed directory |
| `forget` | The array of the forget groups |
| `check` | The `summary` message |
| `init` | The `initialized` message |
| `version` | The restic version |
| `cat config` | The repository configuration |
| `prune` | Nothing |
| `unlock` | Nothing |

The options `backup`, `restore`, `snapshots`, `forget`, `check` and `init` write their JSON only with the flag `--json`. The command `version` writes a text line without `--json`. The command `cat config` always writes the configuration.

## Environment variables

| Variable | Meaning | Default |
|---|---|---|
| `FAKE_RESTIC_EXIT` | The exit code | `0` |
| `FAKE_RESTIC_VERSION` | The restic version that `version` reports | `0.19.1` |
| `FAKE_RESTIC_PROGRESS_RATE` | The `status` messages for each second of a `backup` or a `restore` | `0` |
| `FAKE_RESTIC_PROGRESS_COUNT` | The number of `status` messages | `3` |
| `FAKE_RESTIC_ARGV_FILE` | The file for the received argument vector | none |
| `FAKE_RESTIC_SCENARIO` | The scenario file that the binary reads | none |
| `FAKE_RESTIC_LS_COUNT` | The number of entries that `ls` prints | `3` |

A rate of `0` writes the messages without a wait.

## Scenario file

The variable `FAKE_RESTIC_SCENARIO` names a JSON file. The file selects the duration, the progress rate, a failure, a stale lock and the behaviour on a cancel signal. The file is JSON, because the workspace has no YAML crate (ADR-0019 keeps `serde-saphyr` in the control plane).

The binary reads the values in this order:

1. The default value of the binary.
2. The value of the scenario file.
3. The value of the environment variable `FAKE_RESTIC_EXIT`, `FAKE_RESTIC_PROGRESS_RATE` or `FAKE_RESTIC_PROGRESS_COUNT`.

An environment variable of step 3 overrides the scenario file.

### Structure

The file has two objects:

- `commands`: one entry for each command. The key is the command name: `backup`, `restore`, `forget`, `check`, `snapshots`, `prune`, `unlock`, `init`, `version` or `cat_config`.
- `command_sequence`: an optional array for each command. The first call uses the first entry, the second call uses the second entry, and so on. When the array is shorter than the call count, the `commands` entry applies.

The binary counts the calls in a state file next to the scenario file, `<scenario>.state.json`. A test with its own directory has its own counter. Parallel tests do not meet.

### Fields

| Field | Meaning | Default |
|---|---|---|
| `duration_ms` | The time of the command in milliseconds | `0` |
| `progress_rate` | The `status` messages for each second | `0` |
| `progress_count` | The number of `status` messages when `duration_ms` is `0` | `3` |
| `exit_code` | The exit code of the command | `0` |
| `stderr_lines` | The lines that the binary writes to stderr before the command | `[]` |
| `fail_at_step` | The command stops after this `status` message and writes no summary | none |
| `fail_at_percent` | The command stops when the progress reaches this percent | none |
| `partial_output` | `true`: the command writes the `status` messages and no summary | `false` |
| `lock` | The lock mode: `none`, `manage` or `stale` | `none` |
| `cancel` | The behaviour on a cancel signal | see below |

A `duration_ms` of `0` and a `progress_rate` of `0` write the messages of `progress_count` without a wait.

The `cancel` object has two fields:

| Field | Meaning | Default |
|---|---|---|
| `ignore` | `true`: the binary ignores the cancel signal and continues | `false` |
| `delay_ms` | The wait in milliseconds after the cancel signal, before the exit with code 130 | `0` |

### Lock modes

- `none`: the command does not touch the lock directory.
- `manage`: the command writes a lock file and removes it at a clean end. A present lock file makes the command exit with code 11 and the message of a locked repository.
- `stale`: the command writes a lock file and leaves it, then exits with code 11.

The lock directory is the `locks` directory of the `--repo` option. The command `unlock` removes every lock file. A hard kill removes no lock file, so the lock stays for a later `unlock` (AC-SC-06).

### Examples

A long backup with progress, for a cancel test:

```json
{"commands": {"backup": {"duration_ms": 30000, "progress_rate": 10}}}
```

The second `backup` call fails with exit code 1:

```json
{"command_sequence": {"backup": [{"exit_code": 0}, {"exit_code": 1}]}}
```

A failure at 50 percent of the backup:

```json
{"commands": {"backup": {"progress_count": 10, "fail_at_percent": 50, "exit_code": 1}}}
```

A partial output and then a crash:

```json
{"commands": {"backup": {"progress_count": 3, "partial_output": true}}}
```

A stale lock:

```json
{"commands": {"backup": {"lock": "stale"}}}
```

A slow exit after a cancel signal:

```json
{"commands": {"backup": {"duration_ms": 30000, "progress_rate": 10, "cancel": {"delay_ms": 5000}}}}
```

The binary ignores the cancel signal:

```json
{"commands": {"backup": {"duration_ms": 30000, "progress_rate": 10, "cancel": {"ignore": true}}}}
```

Five locked runs and then a success (AC-SC-05):

```json
{"command_sequence": {"forget": [
  {"exit_code": 11}, {"exit_code": 11}, {"exit_code": 11},
  {"exit_code": 11}, {"exit_code": 11}, {"exit_code": 0}
]}}
```

## Cancel signals

On Unix the binary handles SIGINT and SIGTERM. On Windows the binary handles the console events Ctrl+C and Ctrl+Break. On a signal the binary writes `signal interrupt received, cleaning up` to stderr and exits with code 130 after `cancel.delay_ms`.

An `ignore` of `true` keeps the process alive after the signal. The test then kills the process tree and observes the stale lock of AC-AG-07.

## Password

The binary reads the repository password from `RESTIC_PASSWORD`, `RESTIC_PASSWORD_FILE` or the option `--password-file`. A command that opens a repository stops with exit code 1 when it finds no password. The binary never reads a password from an argument. The binary never reads `RESTIC_PASSWORD_COMMAND`, because a value from a job must not start a program (ADR-0021). restic has no `--password` option, so the binary rejects it as an unknown flag.

## Exit codes

The variable `FAKE_RESTIC_EXIT` selects the code. The known restic codes are `0`, `1`, `3`, `10`, `11`, `12` and `130`. Any other code is a generic failure. When the code is not `0` and the call has the flag `--json`, the binary writes an `exit_error` message to stderr. This behavior follows FR-AG-09.

## Argument vector file

When `FAKE_RESTIC_ARGV_FILE` is set, the binary writes the received arguments to that file as a JSON array. The binary writes the file before it reads the command line, so a test can read the arguments of a refused call. The file never holds an environment value.

## Tests

```sh
cargo test -p restichub-fake-restic
```

The tests in `tests/cli.rs` run the built binary. The golden files are in `tests/golden/`. The tests in `tests/scenarios.rs` cover the scenario file. The tests in `tests/cancel.rs` send a cancel signal to the process group, as the agent does. The names of the tests refer to an acceptance criterion where the test supports one.
