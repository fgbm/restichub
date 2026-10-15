# ResticHub

ResticHub is a central control plane for [restic](https://restic.net) backups on a fleet of Windows, Linux and macOS hosts.

> **Status:** design phase. There is no usable release yet. The requirements, acceptance criteria and architecture decisions are in [`design/`](design/).

## What ResticHub does

- The control plane keeps all profiles, schedules and secrets.
- The control plane decides when a job starts. Agents do not keep schedules.
- A thin agent on each host connects to the control plane, receives signed jobs, runs restic and sends back the results.
- The agent gets repository secrets for the duration of one job only. The agent does not store them.
- Scripts from the control plane run only on a host with the local trust mode. The control plane cannot change that mode.
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

## Run locally

The Compose file at the repository root starts a development stack: PostgreSQL, the database migrations and the control plane with the embedded web UI. You must have Docker.

1. Create `.env` with random development secrets:

   ```sh
   ./scripts/dev-env.sh
   ```

2. Build and start the stack:

   ```sh
   docker compose up --build
   ```

3. Open `http://127.0.0.1:8080`.

4. Read the one-time setup token from the control plane log:

   ```sh
   docker compose logs cp
   ```

   The token is on the line with `setup_token`. Use it on the setup page to create the first administrator. The administrator is the Admin of the tenant `default` in the default single mode, or the Platform Admin in multi mode. Set `RESTICHUB_TENANCY_MODE=multi` to run the multi-tenant mode.

5. Stop the stack and remove the data volume:

   ```sh
   docker compose down -v
   ```

The Compose file is for development only. It turns on insecure cookies and sets development passwords. Read [`ui/README.md`](ui/README.md) for the details.

## Contribute

Read [CONTRIBUTING.md](CONTRIBUTING.md) before you open a pull request. All participants must obey the [Code of Conduct](CODE_OF_CONDUCT.md).

To report a security problem, read [SECURITY.md](SECURITY.md). Do not open a public issue for it.

## License

ResticHub is licensed under the [Apache License 2.0](LICENSE).

ResticHub is not part of the restic project. The restic project does not endorse ResticHub.
