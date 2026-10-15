---
title: Allow-list hooks
description: How a host administrator lists the programs that a profile can run before and after a backup on a host.
---

An allow-list hook is a program that a host administrator lists in a file on the host. A profile refers to the hook by its ID and sends typed parameters. The agent starts only the programs in the file.

Use this mode when you do not want the control plane to send code to a host. A host with the [trust mode](scripts/) can also run server scripts. A profile can use both kinds.

## The allow-list file

| System | File |
|---|---|
| Linux | `/etc/restichub-agent-hooks.toml` |
| Windows | `%ProgramData%\ResticHub\Agent-hooks.toml` |

The agent trusts the file only when root or Administrators own it. No other account may write the file, the program that it lists or a directory above them. If the file is not safe, the agent runs no hook and reports the state `insecure`. A job, the control plane and a self-update cannot change the file.

The agent reads the file again for each job. A change applies to the next job.

## Format

The file is in TOML. An unknown key is an error, so a typo never loosens a rule.

```toml
[[hook]]
id = "pg_dump"
executable = "/usr/bin/pg_dump"
args = ["--format=custom"]
timeout_seconds = 600
phases = ["before"]

[[hook.parameter]]
name = "database"
type = "string"
pattern = "[a-z_][a-z0-9_]*"
required = true
pass_as = "positional"
position = 1

[[hook.parameter]]
name = "password"
type = "secret_ref"
pass_as = "env"
env_name = "PGPASSWORD"
```

Each hook has these keys:

| Key | Meaning |
|---|---|
| `id` | The name that a profile uses. 1 to 64 characters from `a-z`, `0-9`, `.`, `_` and `-`. |
| `executable` | The absolute path of the program. The agent starts it without a shell. |
| `args` | Fixed arguments. |
| `timeout_seconds` | The time limit. The default is 300 seconds. The longest is 24 hours. |
| `working_dir` | The working directory. The default is the directory of the program. |
| `phases` | The phases where the hook may run. An empty list allows every phase. |
| `env` | Fixed environment variables. |

A parameter has a type: `string`, `integer`, `enum`, `boolean` or `secret_ref`. A string can have `max_length` and a `pattern` that the whole value must match. An integer can have `minimum` and `maximum`. An enum lists its `values`. The key `pass_as` says how the value reaches the program: `flag`, `positional` or `env`. A secret always uses `env`.

## Use a hook in a profile

The agent reports the ID, the phases and the parameter types of each hook, and never a path. The profile form offers the hooks that all target hosts report. The control plane does not save a profile with a hook that a target host lacks. The error names the host and the hook.

The phases are `before`, `after`, `on_success`, `on_error` and `finally`. They follow the same rules as the phases of [server scripts](scripts/). A failed `before` hook stops the backup.

Each hook is a step of the job with its own log, like a script.
