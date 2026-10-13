# restichub-emulator

The `restichub-emulator` crate is the agent emulator for the load tests (bench S2). One process runs N emulated agents over the protocol in `proto/`. The crate is the load generator of issue #33.

## The protocol loop

The emulator connects to a control plane, sends `AgentHello` and answers `Ping` with `Pong` (FR-ID-07). It verifies each job with `restichub-proto` before it runs a step (ADR-0005, ADR-0020). A job with a bad signature, a different addressee or an expired time gets `JobRejected` with the reason. A valid job gets `JobAck`, then the step progress, the step results and the job result. A `CancelJob` stops the running step (FR-AG-10). The agent starts no action on its own (ADR-0002). A dropped stream reconnects with exponential backoff and full jitter (FR-ID-08).

## Identity before epic #1

The real agent gets its certificate from the enrollment endpoint (ADR-0026, epic #1). Until that lands, the emulator makes its own certificate authority and one client certificate for each agent with `rcgen`. The material stays in memory. The flag `--write-certs DIR` writes the test authority, the server files and one client file pair for each agent to `DIR`. Epic #1 replaces this with the identity and the key set from enrollment. The test job signing key has a fixed seed, so the test control plane and the emulator agree without a key exchange.

## The executor

The default executor waits for the step time of a profile. The optional executor spawns `restichub-fake-restic` and maps the exit code to a step status (FR-AG-09). Issue #34 adds the load profile and the metrics.

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
| `--duration SEC` | Stop after SEC seconds. Default: run until Ctrl+C. |

## A local run

The test control plane is behind the `test-util` feature. The feature also builds the `restichub-emulator-test-server` binary. Make the certificates first, then start the test server, then start the emulator.

```sh
cargo build --release -p restichub-emulator --features test-util
./target/release/restichub-emulator --write-certs /tmp/emulator-certs --agents 500
./target/release/restichub-emulator-test-server --certs /tmp/emulator-certs --agents 500 --period 5 &
./target/release/restichub-emulator --certs /tmp/emulator-certs --agents 500 --server https://127.0.0.1:PORT
```

The test server prints its URL at the start. Use that URL in the `--server` option. The test server prints its counters when you press Ctrl+C.

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
