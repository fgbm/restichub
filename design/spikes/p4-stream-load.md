# Spike P4: stream load of one control plane process

Status: complete. Date: 2026-10-12.

Related items: NFR-PERF-01, FR-ID-07, FR-ID-08, AC-ID-06, AC-NFR-01, AC-SC-09. [roadmap.md](../roadmap.md) holds the entry for this spike.

## Question

One control plane process serves the agent streams over tonic gRPC and rustls mTLS. Can the process hold 500, 1000 and 2000 agent `Connect` streams? Can the process dispatch signed jobs with low latency? Can the process survive a restart without a reconnect storm and without false offline alerts?

## Method

The program in [spikes/p4-stream-load/](../../spikes/p4-stream-load/) holds two binaries and a runner script.

`p4-server` implements `AgentService.Connect`. The server requires a client certificate. The server takes the agent identifier from the common name of the client certificate, never from the payload. The server keeps a map of agent_id to sender. The server sends `ServerHello`, sends `Ping` every 30 seconds, and dispatches signed jobs. The job rate follows NFR-PERF-01: one job for each agent every 900 seconds. The server signs each job with an Ed25519 key. The server records the time from the dispatch to the `JobAck`. The server prints counters to stdout as JSON every 10 seconds.

`p4-emulator` spawns N agents as tokio tasks. Each agent holds its own client certificate from a test certificate authority. The emulator makes the authority, the server certificate and the client certificates with `rcgen` at start. Each agent connects, sends `AgentHello`, verifies each job with `verify_job` and `validate_job`, answers `JobAck` and a fake `JobResult`, and answers `Pong`. After a disconnect the agent reconnects with exponential backoff and full jitter. The base is 1 second and the maximum is 60 seconds.

`run.sh` runs the scenarios. The runner sets N to 500, 1000 and 2000. Each run dispatches at the NFR-PERF-01 rate. Each run sends one burst of 100 jobs. The burst measures AC-SC-09. The runner collects the control plane CPU and RSS from `/proc` every second. The restart scenario kills the control plane with `SIGKILL`, waits 5 seconds, and starts the control plane again.

This report uses shortened runs of 120 seconds for each N. The burst starts 60 seconds after the start. The restart scenario uses 500 agents. NFR-PERF-01 asks for a field length of 4 hours.

## Environment

The control plane and the emulator run on one host.

- CPU: 12th Gen Intel(R) Core(TM) i5-1240P
- Logical processors: 16
- RAM: 14 GiB
- Kernel: 7.0.0-38-generic
- File descriptor limit: 524288. The host did not allow a higher value.

## Results

The main table holds the normal runs. The latency columns hold the ack latency. The burst column holds the p95 of the 100-job burst.

| N | connect time (s) | peak connect rate (conn/s) | CPU p95 (%) | RSS max (MiB) | ack p50 (ms) | ack p95 (ms) | ack p99 (ms) | burst p95 (ms) |
|---|---|---|---|---|---|---|---|---|
| 500 | 3.05 | 377 | 1.0 | 28.4 | 0.39 | 0.64 | 0.71 | 2.44 |
| 1000 | 3.12 | 632 | 2.0 | 49.6 | 0.37 | 0.55 | 0.64 | 5.39 |
| 2000 | 3.15 | 1605 | 1.0 | 91.3 | 0.38 | 0.55 | 0.70 | 10.90 |

The restart table holds the restart run for 500 agents. The reconnect time is the time from the second start to the last connection.

| reconnect time (s) | peak connect rate (conn/s) | CPU p95 (%) | RSS max (MiB) | false offline count |
|---|---|---|---|---|
| 39.0 | 95 | 6.0 | 27.4 | 0 |

Every run dispatched the full rate. The server got an ack for each job. The server dropped no job. The agents rejected no job. The reconnect counter inside the grace period equalled N in each run.

## Limitations

- Both processes run on one host. The host has no real network path.
- The host has no TLS inspection proxy.
- The emulator and the control plane share the 16 logical processors. The CPU value is the process CPU, not the host load.
- The emulator does no restic work. A real agent spends its time in restic.
- The runs are 120 seconds. NFR-PERF-01 asks for 4 hours.
- The grace period is 300 seconds. Each run is shorter, so the runner records no `grace_alerts` event. The false offline count uses the reconnect time, which stays below the grace period.
- The ack latency is a loopback value. The value does not include the network.
- Each N has one measurement. The table holds no average.

## Decision proposal

- Backoff: keep the base of 1 second and the maximum of 60 seconds with full jitter. The restart run shows the last agents return after 39 seconds. This value stays inside the 300 second grace period. A lower maximum, for example 30 seconds, shortens the tail. Test the lower maximum in the MVP restart test.
- Keepalive: keep the application heartbeat at 30 seconds. The 2000-agent run spends little CPU on the heartbeat. Add HTTP/2 keepalive for a dead connection.
- Grace period: keep 300 seconds (AC-ID-06). The reconnect time of 39 seconds is far below this value. The control plane must not raise an offline alert before the grace period ends.
- NFR-PERF-01: the targets look reachable. One process served 2000 agents with RSS below 0.1 GiB and CPU below 2 percent. The requirement asks for 2 vCPU and 4 GiB. This host is larger. Run the full scenario on a 2 vCPU instance before the 1.0 milestone.

## Effect on requirements

- FR-ID-07: one process holds 2000 permanent streams with an application heartbeat. The result supports the requirement.
- FR-ID-08: exponential backoff with jitter and a grace period work. The restart run shows no false offline alert.
- NFR-PERF-01: 500 agents use less than 2 percent CPU and about 28 MiB RSS. The 2000-agent target looks reachable on a larger host. A run on a 2 vCPU, 4 GiB instance is necessary.
- AC-ID-06: 500 agents reconnect in about 39 seconds, below 5 minutes. The false offline count is 0.
- AC-NFR-01: CPU and RAM stay far below the limits. The dispatch delay is below 1 ms at p95.
- AC-SC-09: the p95 of 100 at once jobs is below 11 ms. This value is far below 3 seconds. The measurement uses an empty agent, so a real restic start is necessary in the MVP test.
