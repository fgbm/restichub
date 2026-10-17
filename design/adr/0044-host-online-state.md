# ADR-0044: The host online state is an open stream recorded in the database

- Status: accepted
- Date: 2026-10-17
- Requirements: FR-ID-07, FR-ID-08, AC-MN-09, AC-ID-06
- Related: ADR-0018, ADR-0036

## Context

The host list, the host page and the Overview showed a host as online when the control plane saw it in the last 90 seconds (`agents.last_seen_at`). After the stream of an agent closed, the host stayed online on these screens for up to 90 seconds. But the dispatcher already refused jobs for it. The stream registry of an instance cannot give the state. The control plane can run in more than one instance (ADR-0018), and each instance must show the same state. ADR-0036 uses `last_seen_at` for the alert "Host offline" for the same reason.

## Decision

- The instance that holds the stream of an agent records the stream in the database. It writes `agents.stream_id` after the hello and clears it at the close of the stream. The close clears the value only when it is the id of the same stream.
- A host is online when it has a recorded stream and `last_seen_at` is in the last 90 seconds. The host list, the list filter, the host page and the Overview use this rule on each instance.
- The 90-second window is a safety net. An instance that crashes cannot clear the streams that it held, so these hosts become offline when the window ends.
- The stream id is a UUIDv7. A write of the open stream changes the row only when the row holds no stream or an older one. A late write of an older stream thus does not replace a newer stream, and the close of the older stream then clears nothing. Inside one process the order of the ids is strict. Between instances the order follows the instance clocks. A clock difference smaller than the reconnect time of an agent keeps the order correct.
- When the write of the open stream fails, the stream task writes it again at each `last_seen` tick until the write succeeds. The event "host online" goes out once, after the successful write. The event "host offline" goes out only when the close cleared the row.
- The Overview caches its facts for 15 seconds on each instance. A change of the stream state removes the cache of the tenant on the instance that holds the stream. Another instance shows the change when its cache ends.
- The alert "Host offline" does not change. It keeps its own threshold on `last_seen_at` (ADR-0036, default 30 minutes). This threshold is the grace period of FR-ID-08: a CP restart does not raise an offline alert.

## Consequences

- When a stream closes, the screens show the host offline at once, and the dispatcher of the instance agrees.
- The displayed state and the dispatcher of one instance can differ for a short time:
  - After an instance crash, the screens show its hosts online for up to 90 seconds.
  - When the first write of the open stream fails, the screens show the host offline until the next tick (30 seconds).
  - A job that starts on demand on an instance without the stream of the host is refused, although the host is online. This was so before this decision.
- A planned restart of the CP stops the agent listener without a wait for the stream tasks. A stream task that ends before the process stops clears its row, and its host shows offline at once. A stream task that the stop interrupts leaves its row, and its host shows online for up to 90 seconds. When the agent connects again, its new stream replaces the old value. No offline alert opens during the restart.
- The `stream_id` column is the one source of the online state for each instance. A future high availability design reads it and does not add a second source.
