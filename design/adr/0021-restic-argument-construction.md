# ADR-0021: restic argument construction and validation

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-PR-01, FR-PR-10, NFR-SEC-03

## Context

Typed steps still carry strings that become restic arguments. A value that starts with `-` can become a restic option. For example, `--password-command=...` runs a program as LocalSystem or root. This bypasses the rule that the CP cannot run a program on a host (ADR-0037).

## Decision

- The agent puts all positional arguments (paths, snapshot IDs, targets) after `--`.
- The agent passes each option value in the attached form `--flag=value`. The agent never passes an option value as a separate argument.
- The CP validates each value before it signs the job. The agent validates each value before execution. These formats apply:
  - snapshot ID: `^[0-9a-f]{8,64}$` or `latest`
  - duration fields (`keep_within`): `^([0-9]+[yYmMwdDhH])+$`
  - size or percent fields (`max_unused`, `max_repack_size`): `^([0-9]+(\.[0-9]+)?%|[0-9]+[kKmMgGtT]?|unlimited)$`
  - `read_data_subset`: `^([0-9]+/[0-9]+|[0-9]+(\.[0-9]+)?%|[0-9]+[kKmMgGtT]?)$`
  - no NUL or line break characters in any string, except the SFTP private key
  - the SFTP private key allows line breaks and rejects a NUL only, because a PEM key has line breaks
- A step without a known kind causes `UNSUPPORTED_STEP`.
- The agent never sets a restic option or an environment variable that executes a program from job data. Examples: `--password-command`, `RESTIC_PASSWORD_COMMAND` and `-o sftp.command`.

## Consequences

- A string field cannot inject a restic option.
- The CP and the agent share one validation module. The formats stay in one place.
- Some valid restic values can fail validation. The list needs maintenance.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Rely on the typed steps only | The strings inside the steps still reach the restic command line. |
| Sanitize the strings on the agent only | The CP must reject a bad job before it signs the job. The agent cannot repair a bad job. |
