# Roadmap

Status: draft. Date: 2026-10-12.

## Milestone 0: spikes

Each spike gives a short report and a decision. Spikes run before the related implementation.

| ID | Spike | Question | Related requirements |
|---|---|---|---|
| P1 | Cancel restic from a Windows service | Can the agent deliver Ctrl+Break to restic from a service without a console? If not, which helper process is necessary? | FR-AG-10, AC-AG-06 |
| P3 | WinRM from Linux | Does `winrm-rs` work against a real Active Directory with NTLM and Kerberos? | FR-IN-04 |
| P4 | Stream load | Can one control plane process hold 500 to 2000 agent streams and survive a restart without false alerts? | NFR-PERF-01, AC-ID-06, AC-NFR-01 |
| P5 | YAML library | Which maintained YAML library replaces the archived `serde_yaml`? | FR-PF-07 |
| P6 | SFTP key delivery | How can the agent give an SFTP private key to restic without a file on the agent (ssh-agent, pipe)? | FR-RP-01, FR-AG-04, FR-AG-06 |

Spike P2 (MSSQL VDI) is cancelled. See ADR-0016.

- P1: the implementation is ready in [`spikes/p1-windows-cancel/`](../spikes/p1-windows-cancel/). The results need a CI run on GitHub. See the [P1 report](spikes/p1-windows-cancel.md).
- P5: the spike is complete. The recommendation is `serde-saphyr`. See the [P5 report](spikes/p5-yaml-library.md) and [ADR-0019](adr/0019-yaml-library.md) (proposed).

## Milestone 1: MVP

The MVP contains all requirements with priority M. The epics are:

1. Protocol: Protocol Buffers contract, signatures, enrollment, CA.
2. Agent: service, step execution, restic runner, JSON parser, cancellation, outbox.
3. Control plane core: tenants, database schema, secrets, audit.
4. Scheduler and queue: cron, repository barriers, retries, stale lock recovery.
5. Profiles and compiler: flat profiles, variables, validation, preview.
6. Restore: snapshots, file tree, restore to another agent.
7. Monitoring: dashboard, alerts, Prometheus.
8. Web UI: all MVP sections, English and Russian.
9. Installation: scripts with a token, Docker Compose for the control plane.
10. Documentation: installation guide, threat model, resticprofile migration guide.
11. Test benches S1 and S2, fake restic, agent emulator.

## Milestone 2: 1.0

All requirements with priority 1.0. Main items: macOS agent, profile inheritance, packages and signing, remote installation, S3 log storage, analytics, TOTP and OIDC, quotas.
