# ADR-0042: The agent log spool and the log acknowledgements

- Status: accepted
- Date: 2026-10-15
- Requirements: FR-LG-07, FR-PR-07, FR-AG-08, FR-AG-10
- Refines: ADR-0008 (additive protocol change), ADR-0020

## Context

The agent dropped a log chunk that found no room in the stream buffer within a moment. The journal kept only the last 64 KiB of the log. A long backup that lost its connection lost the log in the middle.

## Decision

- The agent writes each log chunk of a job to a spool before it sends the chunk. The spool is one protected file for each job in the `log-spool` directory of the configuration directory. The chunks have passed the scrubber, so the file holds no secret.
- The CP sends a `LogAck` message after it stores a chunk. The message gives the newest stored sequence for each stream. The agent sends the chunks of a stream in order. An acknowledgement therefore covers all earlier chunks that went out, and the agent deletes them. The CP sends one acknowledgement for each job at a short interval, not one for each chunk. It keeps free room in its outbound buffer for pings and cancels. If a chunk fails to store for now, the CP stores no later chunk of that job on that connection. It sends no acknowledgement for that job and closes the connection. The agent then sends the job again from its first unacknowledged chunk, so the lines keep their order. The CP also acknowledges a chunk that it refuses for good. Examples are a chunk that is too large and a job that it does not know. Such a chunk cannot hold back the next ones.
- After a reconnect the agent sends every chunk that is not acknowledged, from the first one. After a restart the agent loads the spool files and does the same. The CP ignores a chunk that it stored before (AC-PR-07). The agent keeps a spool until the CP acknowledges it, also when the journal entry of the job expired. The agent cuts a spool file at the first torn record that a crash left. It syncs a spool file when the job ends and when the process closes the spool. A host crash can therefore lose the newest records of a running job.
- A spool has the cap `RESTICHUB_LOG_SPOOL_MAX_BYTES` for all jobs together (64 MiB by default). The job log limit of 1 MiB still applies to each job. When the cap is reached, the agent drops the new chunk, counts its lines, and later writes one marker line that gives the number. The marker is the last line of the job log when the cap holds until the end.
- A running job only writes its chunks to the spool. One drain task for each connection sends the chunks in order, with bounded waits. It leaves a quarter of the stream buffer free for the other messages of the agent. A slow CP cannot delay a cancel (FR-AG-10). From the attach of a stream until the CP greeting, the chunks wait in the spool as unacknowledged, so their order holds. The job result follows the last log chunk of the job after a bounded wait for the drain task.
- The agent splits a text into chunks of at most 64 KiB, the bound of the CP. The marker line for dropped lines does not name a cause that the agent cannot know.

### Compatibility

- The capability `log-ack-v1` goes in `AgentHello.capabilities` and in `ServerHello.accepted_capabilities`. The agent keeps chunks for the acknowledgement only when the CP greeting lists the capability. The CP sends `LogAck` only to an agent that lists it. Before the greeting, the agent holds the chunks in the spool. A CP without the capability gets the spooled chunks once, and the agent then drops them. The agent keeps the log tail in the job result for such a CP, as before. The protocol minor version does not change, because the change only adds a message and a capability (ADR-0008).
- The job result of an agent that holds a spool for the job carries no log tail. The spool sends the whole log in order. Without the capability, the spool sends what it holds once.
