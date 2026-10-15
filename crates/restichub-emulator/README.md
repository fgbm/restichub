# restichub-emulator

The `restichub-emulator` crate is the agent emulator for the load tests (bench S2). One process runs N emulated agents over the protocol in `proto/`. The crate is the load generator of issue #33 and holds the load profile and the metrics of issue #34.

## The protocol loop

The emulator connects to a control plane, sends `AgentHello` and answers `Ping` with `Pong` (FR-ID-07). It verifies each job with `restichub-proto` before it runs a step (ADR-0005, ADR-0020). A job with a bad signature, a different addressee or an expired time gets `JobRejected` with the reason. A valid job gets `JobAck`, then the step progress, the step results and the job result. A `CancelJob` stops the running step (FR-AG-10). The agent starts no action on its own (ADR-0002). A dropped stream reconnects with exponential backoff and full jitter (FR-ID-08).

## Identity before epic #1

The real agent gets its certificate from the enrollment endpoint (ADR-0026, epic #1). Until that lands, the emulator makes its own certificate authority and one client certificate for each agent with `rcgen`. The material stays in memory. The flag `--write-certs DIR` writes the test authority, the server files and one client file pair for each agent to `DIR`. Epic #1 replaces this with the identity and the key set from enrollment. The test job signing key has a fixed seed, so the test control plane and the emulator agree without a key exchange.

## The executor

The default executor waits for the step time of a load profile. It samples the step time, reports the progress at the profile rate and picks a failure exit code (FR-AG-09). The optional executor spawns `restichub-fake-restic` and maps the exit code to a step status.

## The load profile

The option `--profile FILE` reads a load profile. The file is JSON, because the workspace has no YAML crate (ADR-0019 keeps `serde-saphyr` inside the control plane). The profile drives the agent side of the run. The control plane owns the schedule and the job rate (ADR-0002), so the emulator dispatches no job. The profile sets the agents, the ramp-up, the OS labels, the job time, the failure rate, the progress rate, the reconnect storm, the clock offset and the acknowledgement delay.

When `--profile` is present, the profile overrides `--agents`, `--duration`, `--step-seconds` and `--progress-ticks`. Without `--profile`, the emulator builds a profile from those options.

### Fields

| Field | Meaning | Default |
|---|---|---|
| `agents` | The number of emulated agents | `1` |
| `duration_seconds` | The length of the run | `60` |
| `ramp_up_per_second` | The connect rate. `0` connects every agent at once | `0` |
| `os_mix` | A map of OS label to weight. The label goes into `AgentHello.os` only | `{}` |
| `job_duration` | The bounds of the uniform step time | `0.1` to `0.1` |
| `progress_rate` | The `StepProgress` messages for each second | `0` |
| `failure_rate` | A map of restic exit code to probability | `{}` |
| `reconnect_storm` | A drop of a share of the streams at one time | off |
| `clock_skew_seconds` | The offset of the agent clock, for job validation | `0` |
| `ack_delay_ms` | The wait before the `JobAck` of a slow agent | `0` |

The objects `job_duration` and `reconnect_storm` hold these fields:

| Field | Meaning | Default |
|---|---|---|
| `job_duration.min_seconds` | The lower bound of the step time | `0.1` |
| `job_duration.max_seconds` | The upper bound of the step time | `0.1` |
| `reconnect_storm.at_seconds` | The time after the start of the run | `0` |
| `reconnect_storm.percent` | The share of the agents that drop, in percent | `0` |

The emulator refuses a profile with a bad value and names the field. The failure probabilities must sum to `1` or less. The storm selects the agents by index, so a run is reproducible.

### An example

```json
{
  "agents": 500,
  "duration_seconds": 14400,
  "ramp_up_per_second": 25,
  "os_mix": {"linux": 0.9, "windows": 0.1},
  "job_duration": {"min_seconds": 5, "max_seconds": 30},
  "progress_rate": 2,
  "failure_rate": {"1": 0.02, "3": 0.05},
  "reconnect_storm": {"at_seconds": 600, "percent": 20},
  "clock_skew_seconds": 1.5,
  "ack_delay_ms": 250
}
```

## The metrics

The emulator records the connect time, the job acknowledgement latency, the result latency and the counters of a run. The option `--metrics-addr ADDR` serves the counters as a Prometheus text endpoint. The default address is `127.0.0.1:9100`. The value `none` turns the endpoint off. The option `--summary FILE` writes the final JSON summary.

The endpoint holds these metrics:

| Metric | Type | Meaning |
|---|---|---|
| `restichub_emulator_connect_seconds` | summary | The time from the agent start to the established stream |
| `restichub_emulator_ack_seconds` | summary | The time from the job receipt to the `JobAck` |
| `restichub_emulator_result_seconds` | summary | The time from the `JobAck` to the `JobResult` |
| `restichub_emulator_connects_total` | counter | The established streams |
| `restichub_emulator_reconnects_total` | counter | The streams that came back after a drop |
| `restichub_emulator_rejected_total` | counter | The jobs that the agents refused |
| `restichub_emulator_jobs_acked_total` | counter | The acknowledged jobs |
| `restichub_emulator_results_total` | counter | The reported job results |
| `restichub_emulator_streams` | gauge | The agent streams that are open now |

Each summary metric gives the p50, the p95 and the p99. The histogram uses fixed buckets, so a percentile is the upper bound of its bucket. The crate uses no histogram dependency: the workspace lints refuse `unwrap` and `expect`, and every constructor of `hdrhistogram` returns a `Result`.

The summary file holds the same counters and the p50, the p95, the p99 and the maximum of each latency.

### Which AC-NFR-01 numbers come from where

AC-NFR-01 asks for a control plane CPU below 70 percent, a RAM below 3 GB and a dispatch delay below 3 seconds.

- The control plane CPU and the RAM come from the control plane side of bench S2 (#37). The runner reads them from `/proc`. The emulator does not report them.
- The dispatch delay comes from the control plane side too (#37, #38): the control plane measures the time from the dispatch to the `JobAck`. The emulator reports the agent-side acknowledgement latency, which is a lower bound of the dispatch delay and excludes the network.
- The emulator supplies the job load and the agent-side metrics. The bench uses the connect time, the reconnect count and the agent-side latency to explain the run.

## Options

| Option | Meaning |
|---|---|
| `--agents N` | Number of emulated agents. Default 1. |
| `--server URL` | Control plane URL. Default `https://127.0.0.1:50051`. |
| `--server-name NAME` | TLS server name. Default `localhost`. |
| `--ca FILE` | Certificate authority that verifies the server. |
| `--certs DIR` | Directory with `ca.pem`, `agents.txt` and `clients/`. |
| `--write-certs DIR` | Make the test certificates in `DIR` and exit. |
| `--executor NAME` | `simulated` (default) or `fake-restic`. |
| `--fake-restic PATH` | Path to the `restichub-fake-restic` binary. |
| `--tenant ID` | Tenant identifier. Default `tenant-1`. |
| `--key-id ID` | Job signing key identifier. Default `job-key-1`. |
| `--key HEX` | Job signing public key, 32 bytes in hexadecimal. |
| `--step-seconds SEC` | Wall time of one simulated step. Default 0.1. |
| `--progress-ticks N` | Progress messages for each simulated step. Default 0. |
| `--duration SEC` | Stop after SEC seconds. Default: the profile length, or run until Ctrl+C. |
| `--profile FILE` | Load profile file (JSON). Overrides `--agents`, `--duration`, `--step-seconds` and `--progress-ticks`. |
| `--metrics-addr ADDR` | Prometheus endpoint address, or `none`. Default `127.0.0.1:9100`. |
| `--summary FILE` | Write the final JSON summary to `FILE`. |

## A local run

The test control plane is behind the `test-util` feature. The feature also builds the `restichub-emulator-test-server` binary. Make the certificates first, then start the test server, then start the emulator.

```sh
cargo build --release -p restichub-emulator --features test-util
./target/release/restichub-emulator --write-certs /tmp/emulator-certs --agents 500
./target/release/restichub-emulator-test-server --certs /tmp/emulator-certs --agents 500 --period 5 &
./target/release/restichub-emulator --certs /tmp/emulator-certs --agents 500 --server https://127.0.0.1:PORT
```

The test server prints its URL at the start. Use that URL in the `--server` option. The test server prints its counters when you press Ctrl+C.

Add a load profile and the metrics for a bench run:

```sh
./target/release/restichub-emulator --certs /tmp/emulator-certs \
  --server https://127.0.0.1:PORT --profile profile.json \
  --metrics-addr 127.0.0.1:9100 --summary summary.json
```

## Why no argument parser

The workspace has no argument parser. The crate reads its own arguments and prints the usage on `--help`. Add `clap` only when the workspace adds it.

## Tests

`cargo test -p restichub-emulator` runs the unit tests and the message-loop tests against the in-process test control plane.

| Test | Criterion |
|---|---|
| `ac_id_06_fifty_agents_connect_and_report_results` | AC-ID-06 |
| `ac_pr_01_forged_job_is_refused` | AC-PR-01 |
| `fr_sc_09_cancel_stops_a_running_job` | FR-SC-09, FR-AG-10 |
| `fr_id_08_agents_reconnect_after_a_disconnect` | FR-ID-08 |
| `ac_nfr_01_short_load_run_reports_the_summary` | AC-NFR-01, NFR-PERF-04 |

## Agents of a deployed control plane

The option `--enroll-url` enrolls the agents at a running control plane instead of using test certificates. It needs `--ca FILE` with the installation root CA and one token source: `--token-file FILE` (one token on each line) or `--api-url URL` with `--api-token-file FILE` (the emulator creates one enrollment token for each agent). The control plane accepts 10 enrollments each minute from one address, so the emulator starts 9 each minute (`--enroll-per-minute`). `--identities DIR` keeps the enrolled identities, and a later run enrolls only the missing agents. `--enroll-only` ends the run after the enrollment. `--ramp-up N` starts N agents each second. The load runner of bench S2 is `bench/s2/s2.sh` (see `bench/s2/README.md`).
