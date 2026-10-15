# ADR-0042: The agent log spool and the log acknowledgements

- Status: accepted
- Date: 2026-10-15
- Requirements: FR-LG-07, FR-PR-07, FR-AG-08, FR-AG-10
- Refines: ADR-0008 (additive protocol change), ADR-0020

## Context

The agent dropped a log chunk that found no room in the stream buffer within a moment. The journal kept only the last 64 KiB of the log. A long backup that lost its connection lost the log in the middle.

## Decision

- The agent writes each log chunk of a job to a spool before it sends the chunk. The spool is one protected file for each job in the `log-spool` directory of the configuration directory. The chunks have passed the scrubber, so the file holds no secret.
- The CP sends a `LogAck` message after it stores a chunk. The message gives the newest stored sequence for each stream. The agent sends the chunks of a stream in order. An acknowledgement therefore covers all earlier chunks, and the agent deletes them. If a chunk fails to store, the CP stops the acknowledgements of that job on that connection.
- After a reconnect the agent sends every chunk that is not acknowledged, from the first one. After a restart the agent loads the spool files and does the same. The CP ignores a chunk that it stored before (AC-PR-07).
- A spool has the cap `RESTICHUB_LOG_SPOOL_MAX_BYTES` for all jobs together (64 MiB by default). The job log limit of 1 MiB still applies to each job. When the cap is reached, the agent drops the new chunk, counts its lines, and later writes one marker line that gives the number. The marker is the last line of the job log when the cap holds until the end.
- The live send stays non-blocking. A chunk that finds no room within a moment stays in the spool, and the live path never waits for the order lock. A slow CP cannot delay a cancel (FR-AG-10).

### Compatibility

- The capability `log-ack-v1` goes in `AgentHello.capabilities` and in `ServerHello.accepted_capabilities`. The agent uses the spool only after the CP greeting lists the capability. The CP sends `LogAck` only to an agent that lists it. An agent without the capability, or a CP without it, keeps the earlier behavior. The protocol minor version does not change, because the change only adds a message and a capability (ADR-0008).
- The job result of an agent that holds a spool for the job carries no log tail. The spool sends the whole log in order.
