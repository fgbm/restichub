# Bench S1

Bench S1 is the reference bench of `design/acceptance-criteria.md`, section 1.3. This directory holds its automation: Docker Compose, the setup, the scenarios of section 14.2 and the harness of the long runs.

## What runs

| Part | Image | Role |
|---|---|---|
| `postgres` | `postgres:16` | the database of the control plane |
| `cp` | `restichub-cp` (built from `Dockerfile` in the repository root) | the control plane, single-tenant mode |
| `rest-server` | `restic/rest-server:0.14.0` | the one repository of the bench, without authentication, on the bench network only |
| `agent-a` | `restichub-bench-agent:0.18.1` | Linux agent hosts with restic 0.18.1 (10 by default) |
| `agent-b` | `restichub-bench-agent:0.19.1` | Linux agent hosts with restic 0.19.1 (10 by default) |

Each agent host is a container with the real `restichub-agent` and a real, signed restic release. The agent enrolls itself with a one-time enrollment token: `bench.sh setup` writes one token for each host into `state/tokens`, and each host takes one token. Each host holds a data set in `/data` that changes each minute, so each backup reads new data.

What this bench does not cover:

- The Windows hosts and the Debian arm64 host of section 1.3. See `windows-checklist.md`.
- MinIO (S3) and the local directory repository. The scenarios use the rest-server.
- A second tenant. The bench uses the single-tenant mode (ADR-0024); the scenario 14.2.1 creates the tenant `default` with its first Admin.

## Requirements

Docker with Compose v2, `bash`, `curl`, `jq` and `openssl` on the host. A smoke run of 20 agents needs about 2 GB of RAM and 4 CPU cores while it builds. The first build takes 10 to 20 minutes.

## Start and stop

```sh
cd bench/s1
./bench.sh start      # init, build, up, setup
./bench.sh status     # the agents and the job counts
./bench.sh logs cp    # the log of a service
./bench.sh stop       # stop; the state stays
./bench.sh reset      # remove the containers, the volumes and state/
```

`bench.sh init` writes `bench.env` with random secrets. The setup writes the Admin (`bench` / `bench-password-12345`, bench use only), `state/tokens` and `state/plans.txt`. The web UI is on `http://127.0.0.1:18080`.

Set the environment variables before `bench.sh setup`:

| Variable | Default | Meaning |
|---|---|---|
| `AGENTS_A`, `AGENTS_B` | 10, 10 | hosts with restic 0.18.1 and 0.19.1 |
| `PLAN_PERIOD_MIN` | 15 | minutes between two backups of one host; the start of each host shifts so the hosts do not start together |
| `RETENTION_CRON` | `0 */2 * * *` | schedule of `forget` and `prune` |
| `CHECK_CRON` | `30 3 * * *` | schedule of `check` |
| `NO_PLANS` | 0 | 1: no repository and no plan (the idle hosts of AC-AG-02) |
| `AGENT_MEM_LIMIT` | 128m | memory limit of each agent host |
| `COMPOSE_PROJECT_NAME`, `CP_WEB_PORT`, `CP_AGENT_PORT`, `STATE_DIR` | `restichub-bench-s1`, 18080, 18443, `state/` | the project name, the published ports and the state directory; set all four to run a second bench next to the first |
| `BUILD_NETWORK` | `default` | the network of the image build; use `host` when the build containers have no DNS or no route to the internet |
| `RESTICHUB_CP_IMAGE` | `restichub-cp` | use a prebuilt image of the control plane |
| `BENCH_AGENT_IMAGE` | `restichub-bench-agent` | the name of the agent images; the tag is the restic version |
| `SKIP_BUILD` | 0 | 1: the scenarios of section 14.2 and `run.sh` use the images that exist and build nothing |

## Scenarios of section 14.2

Each scenario starts a fresh bench with one agent host and writes `report.md` into its output directory. Add the reports of 14.2.1 and 14.2.3 to the issue #42 and the report of 14.2.4 to the issue #43. The scenario of this directory writes WARN rows for findings that do not stop the scenario.

```sh
bench/s1/e2e/scenario-14.2.1.sh [OUT]   # a new tenant, from the creation to the first backup
bench/s1/e2e/scenario-14.2.3.sh [OUT]   # recovery after a stale lock; about 40 minutes
bench/s1/e2e/scenario-14.2.4.sh [OUT]   # agent revocation during a job (AC-ID-04); about 1 minute
```

A scenario resets the bench of its project. To run a scenario on a host where a timed run uses the bench, use a separate copy of the repository, so that `bench.env`, `state/` and `out/` are not shared. Set `COMPOSE_PROJECT_NAME`, `CP_WEB_PORT` and `CP_AGENT_PORT` to values that no other bench uses. Set `RESTICHUB_CP_IMAGE` and `BENCH_AGENT_IMAGE` to other image names, so that the build does not replace the images of the timed run:

```sh
COMPOSE_PROJECT_NAME=restichub-e2e CP_WEB_PORT=18090 CP_AGENT_PORT=18453 \
RESTICHUB_CP_IMAGE=restichub-e2e-cp BENCH_AGENT_IMAGE=restichub-e2e-agent \
bench/s1/e2e/scenario-14.2.4.sh
```

The scenario 14.2.3 kills a running backup with SIGKILL. The control plane waits 30 minutes (FR-SC-06) and runs `unlock` when the repository is idle. The scenario checks the lock list of the repository and the next backup. `KILL_TARGET` selects the victim:

| `KILL_TARGET` | Victim | Expected |
|---|---|---|
| `agent` (default) | the agent process of the host; the control plane loses the agent during the job and marks the lock | the job fails, the control plane runs `unlock` after 30 minutes, the next backup succeeds |
| `restic` | the restic process only, as AC-SC-06 words it | see the known gap below |

Known gap: when something other than the agent kills restic with SIGKILL, the agent reports a failed step without the stale lock mark (`possible_stale_lock` is set only after the agent stops restic itself). The control plane then runs no `unlock`. The run with `KILL_TARGET=restic` shows it.

The scenario 14.2.2 needs a Windows host. See `windows-checklist.md`. The scenario 14.2.4 runs on Linux hosts here; repeat it on a Windows host when you run the checklist.

## Timed runs

`run.sh` resets the bench, starts it, runs the collector, stops it at the end of the duration and writes the report. The script runs in the foreground. Start it in `tmux` or with `nohup` on a machine that stays on for the whole run.

```sh
bench/s1/run.sh smoke              # 45 minutes: a check of the bench
bench/s1/run.sh 7d  /data/s1-7d    # AC-SC-04, 7 days, 20 agents
bench/s1/run.sh 24h /data/s1-24h   # AC-NFR-06, 24 hours with the traffic capture
bench/s1/run.sh idle /data/s1-idle # AC-AG-02, 24 hours, no plan, the capture runs too
```

The run machine needs 4 CPU cores, 8 GB of RAM and 10 GB of disk for the 7 day run. Use a machine other than a developer workstation.

The output directory holds:

| File | Content |
|---|---|
| `run.txt` | profile, start, duration, revision, host |
| `samples.csv` | each sample: online agents, jobs by status, load of the control plane and PostgreSQL |
| `jobs.jsonl`, `agents.json`, `alerts.json` | the data at the end |
| `result.json`, `failed-jobs.tsv`, `cp-warnings.log` | the summary, each failed job, the warnings of the control plane |
| `report.md` | the report with the AC-SC-04 verdict |
| `capture/`, `capture-report.md` | AC-NFR-06 only: the capture files and the list of the destinations |

Fill `report-template.md` from `report.md` and add it to the issue: #40 for AC-SC-04, #41 for AC-NFR-06.

### Run on its own host

A run can resume after a restart of the host. Copy the repository to `/opt/restichub`, install `systemd/restichub-bench@.service` into `/etc/systemd/system/` and write one environment file for each run in `/etc/restichub-bench/NAME.env`:

```sh
PROFILE=7d
OUT=/var/lib/restichub-bench/s1-7d
BUILD_NETWORK=host
CAPTURE_SECONDS=86400
STOP_AT_END=1
BENCH_REV=<revision>
```

Then run `systemctl enable --now restichub-bench@NAME`. The containers have restart policies. After a restart of the host or of the service, `run.sh` finds `OUT/load-started-epoch`. It starts the containers and the capture again and waits for the original end time. At the end it writes `report.md`, stops the containers (`STOP_AT_END=1`) and writes `OUT/finished`. A second run next to the first needs its own `COMPOSE_PROJECT_NAME`, `CP_WEB_PORT`, `CP_AGENT_PORT` and `STATE_DIR`. Set `SKIP_BUILD=1` for it, because the images exist.

### AC-SC-04: the 7-day run (issue #40)

The criterion: 0 jobs failed because of a lock, 0 `restic check` errors, no false alert. The verdict of `report.md` counts the first two measures. A person reads the alert list in `alerts.json` and marks each false alert.

If the run stops early, for example because the host restarts, the run is not valid. Start it again from the beginning.

### AC-NFR-06: the traffic capture (issue #41)

`capture.sh` runs a `tcpdump` container in the network namespace of the control plane and of one agent host. It rotates the files each hour. Start it before the agents connect (`run.sh` does so): the report lists the start of each connection, and a connection that was open before the capture has no start. `capture.sh report` lists each source and destination and marks the destinations that the bench does not configure. A person confirms each row marked `NO`.

Configured connections: the agent talks to `cp:8443`, to `rest-server:8000` and to the DNS of the Docker network. The control plane talks to `postgres:5432` and answers on `8443` and `8080`.

## Troubleshooting

| Symptom | Check |
|---|---|
| `bench.sh setup` waits for the agents | `./bench.sh logs agent-a`; the agent log shows the registration error. The token files in `state/tokens` have a lifetime of 24 hours. Run `./bench.sh reset` and start again. |
| An agent host restarts in a loop | the memory limit: raise `AGENT_MEM_LIMIT` |
| The registration fails with a certificate error | the CA file `state/tokens/ca.pem` is empty: delete `state/` and run `bench.sh setup` again |
| A port is busy | set `CP_WEB_PORT` and `CP_AGENT_PORT` |
| The disk is full | `./bench.sh reset`, then `docker volume prune` for the old volumes |
