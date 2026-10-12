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

- P1: the spike is complete. The decision is mechanism B (`AllocConsole` and `CTRL_BREAK_EVENT`). See the [P1 report](spikes/p1-windows-cancel.md).
- P5: the spike is complete. The recommendation is `serde-saphyr`. See the [P5 report](spikes/p5-yaml-library.md) and [ADR-0019](adr/0019-yaml-library.md) (accepted).
- P6: the spike is complete. See the [P6 report](spikes/p6-sftp-key-delivery.md) and [ADR-0022](adr/0022-sftp-deferred-to-1-0.md) (accepted). SFTP is deferred to 1.0.
- P4: the spike is complete. The 2000-agent run used 91 MiB RSS, CPU p95 of 2 percent or less, and an ack p95 below 1 ms. The restart of 500 agents returned all agents in 39 seconds with 0 false offline. Decision proposal: reconnect backoff with a base of 1 second, a maximum of 60 seconds, and full jitter. A lower maximum of 30 seconds is a candidate for the MVP restart test (AC-ID-06). The NFR-PERF-01 targets look reachable. Confirm with the 4-hour run of AC-NFR-01 on bench S2. See the [P4 report](spikes/p4-stream-load.md).

Milestone 0 is complete except the P3 WinRM spike. P3 needs an Active Directory test environment.

## Milestone 1: MVP

The MVP contains all requirements with priority M. [Issue #14](https://github.com/fgbm/restichub/issues/14) tracks the epics.

1. [#4 Control plane core](https://github.com/fgbm/restichub/issues/4): tenants, database schema, secrets, audit.
2. [#13 Test benches and release acceptance](https://github.com/fgbm/restichub/issues/13): benches S1 and S2, fake restic, agent emulator.
3. [#1 Protocol and enrollment](https://github.com/fgbm/restichub/issues/1): Protocol Buffers contract, signatures, enrollment, CA.
4. [#2 Agent core](https://github.com/fgbm/restichub/issues/2): service, step execution, restic runner, JSON parser, cancellation, outbox.
5. [#3 Hook allow-list](https://github.com/fgbm/restichub/issues/3): allow-list file, parameter schemas, hook phases.
6. [#5 Repositories](https://github.com/fgbm/restichub/issues/5): repository management, init and access test, maintenance agent.
7. [#6 Profiles and compiler](https://github.com/fgbm/restichub/issues/6): flat profiles, variables, validation, preview.
8. [#7 Scheduler and queue](https://github.com/fgbm/restichub/issues/7): cron, repository barriers, retries, stale lock recovery.
9. [#8 Restore](https://github.com/fgbm/restichub/issues/8): snapshots, file tree, restore to another agent.
10. [#9 Monitoring, alerts and job logs](https://github.com/fgbm/restichub/issues/9): dashboard, alerts, Prometheus, job logs.
11. [#10 Public API and web UI](https://github.com/fgbm/restichub/issues/10): REST with OpenAPI, SSE, Svelte 5 SPA, English and Russian.
12. [#11 Installation and packaging](https://github.com/fgbm/restichub/issues/11): scripts with a token, Docker Compose for the control plane.
13. [#12 Documentation](https://github.com/fgbm/restichub/issues/12): installation guide, threat model, resticprofile migration guide.

## Milestone 2: 1.0

All requirements with priority 1.0. Main items: macOS agent, profile inheritance, packages and signing, remote installation, S3 log storage, analytics, TOTP and OIDC, quotas.
