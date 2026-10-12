# ResticHub

ResticHub is a central control plane for [restic](https://restic.net) backups on a fleet of Windows, Linux and macOS hosts.

> **Status:** design phase. There is no usable release yet. The requirements, acceptance criteria and architecture decisions are in [`design/`](design/).

## What ResticHub does

- The control plane keeps all profiles, schedules and secrets.
- The control plane decides when a job starts. Agents do not keep schedules.
- A thin agent on each host connects to the control plane, receives signed jobs, runs restic and sends back the results.
- The agent gets repository secrets for the duration of one job only. The agent does not store them.
- Hooks run only from an allow-list on the host. The control plane cannot change that list.
- One installation serves many tenants.

## Main parts

| Part | Technology |
|---|---|
| Control plane | Rust, PostgreSQL |
| Agent | Rust, Windows service, systemd, launchd |
| Agent protocol | gRPC with mTLS, Protocol Buffers |
| Public API | REST, OpenAPI, Server-Sent Events |
| Web UI | Svelte 5 |
| Documentation | Starlight |

## Repository layout

| Path | Contents |
|---|---|
| [`crates/`](crates/) | Rust workspace: agent, control plane, profile compiler, protocol, shared code |
| [`docs/`](docs/) | User documentation site (Starlight) |
| [`design/`](design/) | Discovery, requirements, acceptance criteria, architecture decision records, style guides |
| [`styles/`](styles/) | Vale rules for documentation |

## Build

You must have Rust 1.96 or later.

```sh
cargo build --workspace
cargo test --workspace
```

To build the documentation site, you must have Node.js 22 or later.

```sh
cd docs
npm ci
npm run build
```

The published documentation site is at https://fgbm.github.io/restichub/.

## Contribute

Read [CONTRIBUTING.md](CONTRIBUTING.md) before you open a pull request. All participants must obey the [Code of Conduct](CODE_OF_CONDUCT.md).

To report a security problem, read [SECURITY.md](SECURITY.md). Do not open a public issue for it.

## License

ResticHub is licensed under the [Apache License 2.0](LICENSE).

ResticHub is not part of the restic project. The restic project does not endorse ResticHub.
