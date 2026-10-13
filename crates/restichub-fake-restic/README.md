# restichub-fake-restic

The `restichub-fake-restic` binary emulates the restic command line for the tests of the agent. The binary writes recorded restic JSON output and exits with a selected code. The tests of the agent run it on Linux and on Windows.

The recorded output follows restic 0.19.1. The message shapes come from the restic source at tag `v0.19.1`.

## Commands

| Command | JSON output |
|---|---|
| `backup` | The `status` messages and the final `summary` message |
| `restore` | The `status` messages and the final `summary` message |
| `snapshots` | The array of snapshots |
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

A rate of `0` writes the messages without a wait.

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

The tests in `tests/cli.rs` run the built binary. The golden files are in `tests/golden/`.
