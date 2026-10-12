# Discovery report

Date of the research: 2026-10-12. The facts were true on this date.

## 1. Summary

There is a free position in the market. No tool combines these three functions:

1. A central server that schedules all jobs.
2. A compiler of resticprofile-style profiles into steps for agents without local schedules.
3. Remote installation of agents over WinRM or SSH.

Arkeep and Backrest multihost are the nearest tools. Both do not have these functions.

The principle "the agent has no autonomy" does not protect the data. An agent with a repository password can delete the repository. Security must come from the repository topology and the storage controls (ADR-0003).

## 2. Market

| Product | Architecture | Scheduling | Remote installation | License and maturity |
|---|---|---|---|---|
| Arkeep | Go server and Go agent, gRPC, own PKI, Vue | Server | No, manual | Apache-2.0. v0.8, 181 stars, one maintainer, before 1.0 [2][3] |
| Backrest | Each node is a full Backrest. Multihost sync is experimental since 1.13 (2026-05-04) | Client. The server schedules only `prune` and `check` for shared repositories | No, manual pairing token | GPL-3.0. 7.5k stars, v1.14.1 (2026-07-12) [4][5][6] |
| Zerobyte | One instance, no agents, backs up mounted volumes | Instance | Not applicable | AGPL-3.0. 7.0k stars. No fine-grained RBAC [7] |
| Pluton | One instance or a desktop application | Instance | Not applicable | Apache-2.0. 850 stars [1] |
| Fulgurite | One instance, runs restic over SSH | Web server cron | SSH keys | Polyform Noncommercial |
| Restic Explorer | Monitoring of repositories only | Scheduled scans | Not applicable | MIT. Version 1.0 on 2026-04-04 |
| resticprofile | Configuration wrapper, no server | Operating system scheduler | Not applicable | BSD. The reference for the profile model |
| Kopia repository server | Clients do not know storage credentials | Client | Not applicable | Apache-2.0. Mature. A reference for the secret model [12] |

**Demand.** In Backrest issue #68, users ask in 2026 for agent auto-enrollment, shared plans and full server-side management [6]. The Backrest maintainer keeps client-side scheduling by design.

**Gaps that ResticHub fills:**

- Server-owned scheduling with a profile compiler.
- Remote installation of agents.
- Windows and VSS as primary use cases.
- Secrets delivered for one job only.
- Real RBAC and multi-tenancy.

## 3. restic facts that affect the design

### 3.1. Versions

| Version | Date |
|---|---|
| 0.17.0 | 2024-07-26 |
| 0.18.0 | 2025-03-27 |
| 0.18.1 | 2025-09-21 |
| 0.19.0 | 2026-06-09 |
| 0.19.1 | 2026-07-05 |

The minimum supported version is 0.18.1. The target version is 0.19.1 [8]. Version 0.18 added `check --json` and JSON fatal errors (`exit_error`). Version 0.19 changed exit code semantics: code 3 for missing source paths and for `forget` failures, code 130 for SIGINT.

### 3.2. JSON output and exit codes

The restic documentation says that the JSON output stays backward compatible. New message types and fields can occur [9]. `unlock`, `copy`, `rewrite` and `repair` have no documented JSON output. `dump` writes raw data.

| Code | Meaning | Since |
|---|---|---|
| 0 | Success | |
| 1 | Command failed | |
| 3 | Some source data could not be read, or `forget` could not remove snapshots | Extended in 0.19.0 |
| 10 | Repository does not exist | 0.17.0 |
| 11 | Could not lock the repository | 0.17.0 |
| 12 | Wrong password | 0.17.1 |
| 130 | Cancelled | SIGINT since 0.19.0 |

An unknown exit code must be treated as a failure [9].

### 3.3. Locks

| Lock | Commands |
|---|---|
| Exclusive | `forget`, `prune`, `check`, `tag`, `rewrite`, `key passwd`, `key remove`, `repair`, `recover`, `migrate` |
| Append | `backup`, `key add`, `copy` (destination) |
| Read | `restore`, `ls`, `find`, `dump`, `snapshots`, `stats`, `diff`, `copy` (source) |
| None | `unlock` |

A lock is stale after 30 minutes, or immediately if the process on the same host is dead. restic refreshes locks every 5 minutes. `--retry-lock` retries with backoff. `unlock --remove-all` removes live locks too and is dangerous [10].

**Effect:** the control plane must serialize exclusive operations for each repository (FR-SC-04). This also solves a frequent operations problem: a lock that stays for days after an interrupted job.

### 3.4. Security

- All keys of a repository open the same master key. There are no key permissions [10].
- rest-server `--append-only` stops deletion. `forget` and `prune` must then run from a trusted host that clients cannot reach. Use `--keep-within` on such repositories, because a compromised client can create false snapshots [11].
- restic has no native S3 Object Lock support.

### 3.5. Cancellation

- On Unix, SIGINT and SIGTERM let restic remove its locks.
- On Windows, Go receives only Ctrl+C and Ctrl+Break. A Windows service has no console. Delivery of Ctrl+Break from a service needs a test (spike P1).
- After a hard kill, the lock stays for up to 30 minutes. The repository stays consistent.

### 3.6. Library use

restic has no official Go library API. Issue #4406 is open [13]. rustic_core 0.13 is in an early stage. rustic prune does not obey restic locks. Do not mix restic and rustic on one repository [14].

## 4. Technology facts

| Area | Fact |
|---|---|
| gRPC | `tonic` 0.14.6, mature. `rustls` 0.23.45. |
| ConnectRPC | connect-rust joined the ConnectRPC project on 2026-08-26. Version 0.9, before 1.0. It passes the full conformance suite and supports bidirectional streaming [15][16]. |
| CA | `rcgen` 0.14.10. |
| Windows service | `windows-service` 0.8.1. |
| WinRM | `winrm-rs` 1.2.2: NTLMv2, Kerberos through `cross-krb5`. About 6000 downloads, one maintainer. |
| SSH | `russh` 0.64.1, asynchronous, pure Rust. |
| Queue | apalis-postgres 1.0 published on 2026-10-12. pgmq needs an extension. |
| Cron | `croner` 4.0.1 with time zones and DST handling. |
| Self-update | `self_update` 1.3.0 verifies signatures but has no rollback. |
| Object storage | `object_store` (Apache Arrow): S3, Azure, GCS, local. |
| UI | Svelte 5.57, SvelteKit 3.0.1 (2026-10-06). |
| Code signing | SignPath Foundation gives free Authenticode signing to open source projects with a public build [17]. |

## 5. Lessons from similar systems

- Velociraptor CVE-2026-5329: authenticated clients could write to server queues and get remote code execution. The server must take the agent identity from the certificate, never from the payload.
- SaltStack CVE-2020-11651: missing method validation gave command execution on all minions. A central server must not be able to run arbitrary commands.
- Nomad and GitLab Runner: reconnect storms and polling load need backoff with jitter and a grace period.

## 6. Sources

1. The 4 Best restic GUI Tools in 2026, Pluton blog. https://usepluton.com/blog/best-restic-gui-2026
2. Arkeep repository. https://github.com/arkeep-io/arkeep
3. Arkeep releases. https://github.com/arkeep-io/arkeep/releases
4. Backrest repository. https://github.com/garethgeorge/backrest
5. Backrest multihost documentation. https://garethgeorge.github.io/backrest/docs/multihost
6. Backrest issue #68, centralized backup management. https://github.com/garethgeorge/backrest/issues/68
7. Zerobyte repository. https://github.com/nicotsx/zerobyte
8. restic changelog. https://raw.githubusercontent.com/restic/restic/master/CHANGELOG.md
9. restic scripting documentation, v0.19.1. https://raw.githubusercontent.com/restic/restic/v0.19.1/doc/075_scripting.rst
10. restic design document, v0.19.1. https://raw.githubusercontent.com/restic/restic/v0.19.1/doc/design.rst
11. restic forget documentation, append-only mode, v0.19.1. https://raw.githubusercontent.com/restic/restic/v0.19.1/doc/060_forget.rst
12. Kopia repository server. https://kopia.io/docs/repository-server/
13. restic issue #4406, Go library. https://github.com/restic/restic/issues/4406
14. rustic FAQ. https://rustic.cli.rs/docs/FAQ.html
15. connect-rust joins the Connect project, Buf blog, 2026-08-26. https://buf.build/blog/connect-rust-joins-the-connect-project
16. connect-rust repository. https://github.com/connectrpc/connect-rust
17. SignPath Foundation. https://signpath.org

## 7. Not verified

- VSS support in Arkeep and Backrest.
- Delivery of SIGTERM to restic on Windows.
- Current status of S3 Object Lock in restic.
- Quantitative memory figures for restic on large repositories.
