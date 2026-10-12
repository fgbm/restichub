# ADR-0011: restic integration through the CLI and JSON output

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-AG-06, FR-AG-08, FR-AG-09, FR-AG-11, NFR-CMP-03

## Context

restic has no official Go library API. The maintainers decline to expose internal packages (restic issue #4406). The documented integration is the CLI with `--json`. The restic documentation says that the JSON output stays backward compatible, but new fields and message types can occur.

rustic_core is a Rust implementation of the restic repository format. Its README says that it is in an early stage. rustic prune is lock-free and does not obey restic locks. `restic prune` on a repository where rustic writes can lose data.

## Decision

- The agent starts the restic binary as a child process and parses its JSON output.
- The agent package contains a tested restic version. The minimum supported version is 0.18.1. The target version is 0.19.x.
- The parser ignores unknown fields and message types.
- The agent maps exit codes as in FR-AG-09. An unknown code is a failure.
- The agent hides the restic call behind a Rust trait. A future implementation can use rustic_core.
- ResticHub never mixes restic and rustic on one repository.

## Consequences

- We depend on the stability of the restic CLI.
- Some commands have no JSON output (`unlock`, `copy`, `rewrite`, `repair`). The agent parses their text output or reports only the exit code.
- A compatibility test suite runs for each supported restic version.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| rustic_core as the engine | Early stage. Lock model is not compatible with restic. |
| Go wrapper around restic internals | Not supported by the restic maintainers. |
