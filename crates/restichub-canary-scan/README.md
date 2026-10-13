# Canary secret scan

The tool searches a canary marker in each location where a secret must not occur. The end-to-end
tests and the benches use the tool. The requirements are NFR-SEC-04 and NFR-SEC-05.

The tool never prints the canary value. A finding shows the target, the byte offset, the line, the
encoding and a masked snippet.

## Build

```sh
cargo build -p restichub-canary-scan
```

## Run

```sh
restichub-canary-scan --canary CANARY-7f3a91c2d4e5 --file /var/log/restichub/cp.log
```

The tool gives one line for each finding.

```
found target=/var/log/cp.log offset=42 line=3 encoding=raw snippet=job 12 [REDACTED] done
```

## Canary input

| Option | Function |
|---|---|
| `--canary VALUE` | Add one canary value. Repeat the option for more values. |
| `--canary-file PATH` | Read the canary values from a file, one in each line. An empty line and a line that starts with `#` are ignored. |

The option `--canary` puts the value in the process arguments of the tool. A command-line scan then
finds the tool itself. Use `--canary-file` in an automatic run.

## Targets

The tool scans each target. At least one target is necessary.

| Option | Location | Acceptance criterion |
|---|---|---|
| `--file PATH` | A log file, a container log (the output of `docker logs`) or any other file. | AC-LG-04 |
| `--dir PATH` | A directory tree, for example the agent state, journal and configuration directory. Symlinks are not followed. | AC-AG-03, AC-RS-03 |
| `--database URL` | A PostgreSQL dump of every table as text. The tool runs `pg_dump` with the URL. | AC-RP-02 |
| `--cmdline` | The process command lines of `/proc` on Linux. | AC-AG-04 |
| `--cmdline-file PATH` | A file that holds the process command lines. | AC-AG-04 |

The tool passes the database URL to `pg_dump` through the environment, so the password never
reaches the process arguments.

`--cmdline` needs Linux. On Windows, collect the command lines into a file and use
`--cmdline-file`. Use these commands for the collection:

```sh
# Linux
for path in /proc/[0-9]*/cmdline; do tr '\0' ' ' < "$path"; echo; done > procs.txt
```

```powershell
# Windows
Get-CimInstance Win32_Process | Select-Object -ExpandProperty CommandLine | Set-Content procs.txt
```

## Encodings

The tool searches each byte form that one value can take.

| Encoding | Example for the value `AB` |
|---|---|
| Raw bytes and UTF-8 | `AB` |
| UTF-16LE | `41 00 42 00` |
| base64, standard alphabet | `QUI=` |
| base64, URL-safe alphabet | `QUI=` |
| Hexadecimal, lower and upper case | `4142`, `4142` |
| Percent-encoding, upper and lower case | `AB` |
| JSON escaping | `AB` |

The canary marker format is `CANARY-<uuid>`. Every character of this marker is a letter, a digit or
a hyphen. For this marker the percent-encoding and the JSON escaping agree with the raw bytes, so
the two forms collapse into one.

### Limits

1. The search is exact. The tool does not decode a target and does not search an arbitrary
   transformation.
2. A canary inside a larger base64 blob starts at group offset 0, 1 or 2. The tool searches the
   three offsets. The last base64 character of a canary depends on the bytes after the canary when
   the canary length is not a multiple of three. The tool leaves this character out. A shorter exact
   match is enough for a finding.
3. UTF-16BE and other byte orders are not searched.
4. A canary that spans two process arguments or two log records is not found.

## Exit codes

| Code | Result |
|---|---|
| 0 | No finding. The location is clean. |
| 1 | A finding. A secret reaches a location where it must not occur. |
| 2 | A usage error or an input/output error. |

## Bench commands

Bench S1 (#36):

```sh
restichub-canary-scan \
  --canary-file .canary \
  --database "$RESTICHUB_TEST_DATABASE_URL" \
  --dir "$AGENT_STATE" \
  --file "$CP_LOG" \
  --file "$CONTAINER_LOG" \
  --cmdline
```

Bench S2 (#37):

```sh
restichub-canary-scan --canary-file .canary --file "$EMULATOR_LOG" --cmdline-file procs.txt
```

## Library use

A Rust test scans a database dump or a log buffer with the same search:

```rust
let scanner = restichub_canary_scan::Scanner::new(&[restichub_canary_scan::Canary::parse(canary)]);
assert!(scanner.scan("dump", dump.as_bytes()).is_empty());
```
