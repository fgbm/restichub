# Bench S2: load bench for 500 agents

Bench S2 checks AC-NFR-01 and the restart part of AC-ID-06. The control plane runs in a container that has the limits of a 2 vCPU, 4 GB RAM virtual machine. The agent emulator runs 500 agents on the same host. A script creates one plan for each agent. Thus the control plane starts one job each 15 minutes for each agent (ADR-0002: the agent has no scheduling logic).

## What the run checks

| Check | Target | Source |
|---|---|---|
| CPU of the control plane | 95th percentile below 70 % of 2 vCPU | `docker stats`, sample each 5 s |
| RAM of the control plane | maximum below 3072 MiB | `docker stats`, sample each 5 s |
| Dispatch delay | 95th percentile below 3 s | histogram `restichub_job_dispatch_delay_seconds` |
| Reconnect after a restart | all agents back in 300 s or less, no `agent_offline` alert | `restart-test` |

The samples of the first 300 s after the plans exist are not part of the CPU and RAM figures. In this time the agents connect and the first jobs start.

## Requirements

- A host with Docker and Docker Compose v2. The reference host is a Windows machine with Docker and 128 GB RAM.
- A Bash shell with `curl`, `jq`, `openssl` and `awk`. On Windows use Git Bash.
- The emulator binary `restichub-emulator`. Build it with `cargo build --release -p restichub-emulator`, or copy it from a build machine. The default path is `target/release/restichub-emulator`; set `EMULATOR` for another path.
- Open the ports 8080 and 8443 on the loopback address.

## The whole run

```sh
cd bench/s2
AGENTS=500 DURATION=14400 ./s2.sh run
```

This command does these steps:

1. `up` builds the control plane image and starts PostgreSQL and the control plane with the limits `cpus: 2` and `mem_limit: 4g`.
2. `setup` makes the administrator, an API token and the file `out/ca.pem` with the installation CA.
3. `enroll` creates one enrollment token for each missing agent and enrolls the agents. The control plane accepts 10 enrollments each minute from one address (FR-ID-05), and all emulated agents share the address of the host. The enrollment takes about 56 minutes. It runs once: the identities stay in `out/identities`, and the next run enrolls only the missing agents. The script deletes the token file after the enrollment.
4. `collect` samples the CPU and RAM and reads the metrics each 60 s.
5. `emulate` connects the 500 agents. The simulated executor runs each job in about 6 s. Without `PROFILE`, the agents send no job log.
6. `load` creates 500 repositories, profiles and plans. The plan of agent *n* runs at the minutes *n* mod 15, +15, +30 and +45 of each hour. Thus the 500 jobs spread over the 15 minutes. Set `LOAD_BURST=1` to start all plans in the same minute.
7. `report` writes `out/report.md` and exits with status 1 when a target fails.

Run the steps one by one for a long enrollment or for a rerun: `./s2.sh up`, `./s2.sh setup`, `./s2.sh enroll`, then `./s2.sh collect &`, `./s2.sh emulate &`, `./s2.sh load`. After the run, stop the collector with `touch out/collect.stop` and run `./s2.sh report`.

The 4-hour command for the reference host:

```sh
cd bench/s2 && AGENTS=500 DURATION=14400 OUT=$PWD/out-500-4h ./s2.sh run 2>&1 | tee run-500-4h.log
```

The run takes 56 minutes of enrollment plus 4 hours of load. Keep the host awake, and do not start other heavy work on it.

## Load profile

Set `PROFILE` to a load profile file of the emulator (see `crates/restichub-emulator/README.md`). The profile sets the job time, the progress rate, the job logs and the hostile agents. The script writes one copy of the profile for each emulator process with the agent count of the process. `DURATION` sets the run length, and the options `STEP_SECONDS` and `RAMP_PER_SECOND` have no effect with a profile.

A profile with job logs, for the log load on the database (LT-2, LT-4):

```json
{"job_duration": {"min_seconds": 5, "max_seconds": 30}, "progress_rate": 1,
 "log_lines_per_second": 20, "log_line_bytes": 120}
```

The report then shows the sent and the acknowledged log chunks.

## Hostile agents (PR-SEC-06)

A profile with the object `hostile` makes the first agents send forged job results, a flood of progress messages and one oversize message. Only the first emulator process keeps the hostile agents. For one hostile agent among 500:

```json
{"job_duration": {"min_seconds": 5, "max_seconds": 30},
 "hostile": {"agents": 1, "forged_result_share": 1, "flood_per_second": 1000, "oversize": "above_limit"}}
```

Do these steps:

1. Run the load with this profile: `PROFILE=hostile.json ./s2.sh run`.
2. Read the section "Hostile agents" of `out/report.md`. It gives the forged results, the flood messages and the oversize messages that the emulator sent. It also gives the audit records `agent_message_refused` by reason, the reconnects of the emulator and the state of the control plane at the end.
3. Compare the dispatch delay of the run with a run without hostile agents. The dispatch delay of the other agents must stay below 3 s at the 95th percentile.

The audit limiter of the control plane caps the audit records, so the audit count can be lower than the count of forged results. A message above the protocol limit (8 MiB) closes the stream of the agent, and the agent connects again.

## Several emulator processes and hosts (G-03, G-04)

`PROCS=N` divides the agents among N emulator processes. Each process has its own identity store in `out/identities/pN`, its own log `out/emulator-pN.log` and its own summary `out/emulator-summary-pN.json`. The script adds the summaries into `out/emulator-summary.json`. The counters add up; a merged percentile is the highest percentile of the processes.

One process ran 1000 agents in the smoke test (see "Capacity of one emulator process"). Use more processes when the host has more CPUs than one process uses, or to start the agents on more hosts.

The enrollment limit applies to each source address. `EMULATOR_HOSTS` is a list of SSH targets that enroll the groups at the same time. The word `local` is this host. The script gives the groups to the hosts in turn. For each group, the script copies the emulator, `ca.pem` and the token file to the host, enrolls the agents there and copies the identities back. Then it removes the directory on the host. Each host must reach the agent port of the control plane at `CP_AGENT_URL`. The emulator must run on the host: use the same CPU architecture and a compatible C library.

```sh
PROCS=4 AGENTS=2000 EMULATOR_HOSTS="local user@lxc-1 user@lxc-2 user@lxc-3" ./s2.sh enroll
PROCS=4 AGENTS=2000 ./s2.sh run
```

With 4 addresses, 2000 agents enroll in about 56 minutes instead of 3.7 hours. The `run` command finds the identities and enrolls no agent again. The emulators of `emulate` all run on this host.

### Capacity of one emulator process

| Item | Value |
|---|---|
| Agents | 1000 |
| Control plane | `restichub-emulator-test-server` on the same host |
| Host | Workstation with 16 cores and 14 GiB RAM |
| Run length | 240 s; 100 new agents each second at the start |
| Job load | One job each 5 s for each agent; step time 1 s to 3 s |
| Progress and logs | One progress message each second; 20 log lines of 120 bytes each second |
| Emulator memory (maximum RSS) | 172 MiB |
| Emulator CPU | 19 % of one core on average, 29 % at the maximum |
| Streams | 1000 connects, 0 reconnects, 0 refused jobs |
| Job counts | 47344 acknowledged, 46891 results; acknowledgement p95 0.5 ms or less |
| Logs | 387275 chunks (221 MB); all acknowledged, except 47 chunks that were in flight at the stop |

This load is 180 times the job rate of a 15-minute interval. One process thus has a large margin for 1000 agents, and the split into processes is not necessary for the capacity. The test does not include the enrollment and the CPU of a real control plane.

## Several tenants (G-15, PR-MSP-*)

`TENANTS=N` starts the control plane in multi-tenant mode (ADR-0024) and makes N tenants. Run `./s2.sh down` before you change the mode of an existing bench. These steps occur:

1. `setup` creates the Platform Admin `bench`. The Platform Admin creates the tenants `bench-t1` to `bench-tN`, takes the Admin role in each tenant, and creates one API token for each tenant in `out/tenants/tN/api-token`.
2. `enroll` creates the enrollment tokens of each tenant in a token file and enrolls the agents of the tenant into `out/identities/tN`.
3. `emulate` starts one emulator process for each tenant.
4. `load` creates the repositories, the profiles and the plans of each tenant with the API token of the tenant.
5. `collect` and `report` read the metrics of each tenant and add them up.

`TENANT_AGENTS` sets the size of each tenant. For PR-MSP-01 (5 tenants with 100 agents and 45 tenants with 11 agents):

```sh
TENANTS=50 TENANT_AGENTS="5x100,45x11" ./s2.sh run
```

Then `AGENTS` is the sum of the list. Without `TENANT_AGENTS`, the script divides `AGENTS` evenly. With 50 tenants, 50 emulator processes run on the host.

## Restart test (AC-ID-06)

With the 500 agents connected, run:

```sh
./s2.sh restart-test
```

The script restarts the control plane container, counts the agents that hold a stream through `restichub_agent_up`, and stops the clock when all agents are back. It then checks the alert list for `agent_offline`. The result is in `out/restart-report.md`.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `AGENTS` | 500 | Number of emulated agents |
| `DURATION` | 14400 | Seconds that the emulator runs after the enrollment |
| `INTERVAL_MIN` | 15 | Minutes between the jobs of one agent (a divisor of 60) |
| `OUT` | `bench/s2/out` | Directory for the logs, the CSV files and the report |
| `IDENTITIES` | `$OUT/identities` | Store of the enrolled identities |
| `EMULATOR` | `target/release/restichub-emulator` | Path of the emulator |
| `STEP_SECONDS` | 2 | Wall time of one simulated step |
| `RAMP_PER_SECOND` | 20 | Agents that connect each second at the start |
| `WARMUP_SECONDS` | 300 | Seconds of the load that the report skips |
| `CP_CPUS` | 2 | CPU share for the percentage (matches `cpus` in `compose.yaml`) |
| `CPU_LIMIT_PCT`, `RAM_LIMIT_MB`, `DELAY_LIMIT_SECONDS` | 70, 3072, 3 | Targets |
| `CP_MODE` | `compose` | `native` runs a local `restichub-cp` binary (`CP_BIN`) for a smoke run on a developer machine. `remote` uses a compose stack on another host over SSH (see "Remote control plane") |
| `CP_SSH`, `CP_REMOTE_DIR`, `CP_IMAGE` | none, `/opt/restichub-s2`, none | Remote mode: the SSH target, the directory of the stack on it, and the image of the control plane (use a digest) |
| `CP_WEB_URL`, `CP_AGENT_URL` | `http://127.0.0.1:8080`, `https://localhost:8443` | Where the script and the emulator reach the web port and the agent port |
| `PROFILE` | none | Load profile file of the emulator (see "Load profile") |
| `PROCS` | 1 | Emulator processes in the single-tenant mode |
| `EMULATOR_HOSTS` | `local` | SSH targets that enroll the groups; `local` is this host |
| `REMOTE_EMULATOR_DIR` | `/var/tmp/restichub-s2` | Temporary directory on an enrollment host |
| `TENANTS` | 0 | Number of tenants; above 0 starts the multi-tenant mode |
| `TENANT_AGENTS` | none | Agents of each tenant, for example `5x100,45x11` |
| `TOKEN_TTL_SECONDS` | 86400 | Time to live of an enrollment token |

The native mode needs a PostgreSQL administrator (`psql`, or the container name in `PG_CONTAINER`). Its figures do not count for AC-NFR-01: the process is not limited to 2 vCPU.

## Remote control plane

Use this mode when the control plane runs on a separate virtual machine, as the reference bench S2 requires. The emulator and the script run on another host.

1. Install Docker Engine with the Compose plugin on the virtual machine. Copy `bench/s2` and `crates/restichub-db/sql` to `$CP_REMOTE_DIR` on it, with the same relative layout.
2. Use the firewall of the virtual machine to let only the emulator host reach the agent port 8443. Docker bypasses `ufw`, so put the rule in the `DOCKER-USER` chain.
3. Open an SSH tunnel from the emulator host to the web port: `ssh -N -L 8080:127.0.0.1:8080 <host>`.
4. Set `CP_MODE=remote`, `CP_SSH=<user>@<host>`, `CP_IMAGE=<image>@sha256:<digest>` and `CP_AGENT_URL=https://<host>:8443`, then run `./s2.sh run`. The address in `CP_AGENT_URL` goes into the certificate of the agent listener.

The collector reads the CPU and RAM of the container with `docker stats` over SSH.

## Files in the output directory

| File | Content |
|---|---|
| `resources.csv` | `epoch,cpu_pct,rss_mb` each 5 s |
| `metrics.csv` | `epoch,agents_up,dispatch_count,dispatch_le_limit,jobs_succeeded,jobs_failed` each 60 s |
| `metrics-last.txt` | The last `/metrics` text |
| `report.md` | The checked result, in the layout of `report-template.md` |
| `emulator-<group>.log`, `emulator-summary-<group>.json` | Output and summary of one emulator process; the group is `main`, `pN` or `tN` |
| `emulator-summary.json` | The summaries of all processes, added up (connects, reconnects, results, logs, hostile messages) |
| `tenants/tN/api-token`, `tenants/tN/id` | API token and identifier of a tenant in the multi-tenant mode |

## Manual part on the Windows host

These steps need a person; the script cannot do them.

1. Install Docker Desktop with the WSL 2 backend, or Docker Engine, on the Windows host. Give Docker at least 8 CPUs and 16 GB RAM, so that the emulator and the 2 vCPU container do not compete.
2. Install Git for Windows (Git Bash), `jq` and a Rust toolchain, or copy a built `restichub-emulator.exe` and set `EMULATOR`.
3. Run `./s2.sh run` as shown above in a Git Bash window. Do not let the host sleep.
4. For the check with a real Windows agent, create a Hyper-V virtual machine, install the agent there with `restichub-agent register`, and point it to `https://<host>:8443`. This check is outside the default load run.
5. When the run ends, copy `out/report.md`, `out/resources.csv` and `out/metrics.csv` and add the report to the issue for the acceptance.
