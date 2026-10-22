# ADR-0052: The reconnect grace of the absence tests

- Status: accepted
- Date: 2026-10-22
- Requirements: FR-ID-08, FR-PR-07, FR-MN-03, AC-ID-06, AC-PR-07
- Amends: the section "Alert kinds and state" of [ADR-0036](0036-alerts-and-notification-channels.md), and decision 7 of [ADR-0044](0044-host-online-state.md)

## Context

ADR-0036 decides that the alert "Host offline" reads `agents.last_seen_at` from the database, so each instance gives the same answer. ADR-0044 decides that the threshold of that alert is the grace period of FR-ID-08, so a restart of the control plane raises no offline alert.

A database outage stores no contact of any agent. The first pass after the return therefore read an old `last_seen_at` for every host. It opened the alert "Host offline" for each of them, and it closed the alerts some seconds later when the hosts connected again. The operator got a pair of messages for a fault that never existed. The same old contact made the stale lock recovery fail a running job as `agent_lost` (AC-PR-07).

The registry of the online agents of an instance starts empty and needs the same time to fill after a database outage. The agent connects again with a backoff of 60 seconds at most, so two minutes of grace cover the reconnection.

## Decision

- The control plane judges the absence of an agent only after the reconnect grace of 2 minutes. The grace is `RECONNECT_GRACE`, the wait that the stale lock recovery of AC-PR-07 already uses. It starts with the first pass that read the agents after the start of the process or after a pass that could not read them.
- The dispatcher, the stale lock recovery and the alert evaluation read the same grace of the organization. The alert evaluation has no timer of its own.
- In the grace the alert "Host offline" neither opens nor closes. An alert that is open when the grace starts stays open, and it sends one recovery message after the grace, when its host is back.
- Each instance keeps the grace in its own memory. Two instances can therefore alert a host up to 2 minutes apart. The grace is not in the database, so a restart of an instance starts it again, as FR-ID-08 requires.
- The grace is kept for each organization. A failed read of the list of the organizations restarts the grace of every organization. A failed pass of one organization restarts the grace of that organization only. No organization waits for the failure of another.
- An organization whose pass keeps failing stays in the grace, so its absence tests stay off. The grace of an organization ends only after passes that read its agents for the whole 2 minutes. The control plane warns about a run of failed passes of an organization that lasts 15 minutes. It repeats the warning at that interval while the run continues.
- The threshold of the alert "Host offline" keeps its meaning from ADR-0036 and ADR-0044. The reconnect grace comes on top of it: the alert opens when the threshold and the grace have both passed.

## Consequences

- A host that stays away raises the alert "Host offline" about 2 minutes later than its threshold. This holds after a start of the control plane and after a database outage. A host that comes back in that time raises no alert.
- An alert that is open when the grace starts sends its recovery message after the grace, at most about 2 minutes later than before. It never sends a second open message.
- AC-ID-06 holds: no "agent offline" alert occurs during the grace period. A host that connects later than 2 minutes after the return of the database gets an alert that is false. Bench S2 (500 agents) must show whether all agents connect in that time.
- A fault of one organization delays no alert of another organization, and the log tells the operator when the pass of an organization keeps failing.
- The alert evaluation reads the grace of the instance that runs it. Instances do not disagree about the alert itself: only the moment of the open differs, by less than the grace.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Add the grace to the threshold of the rule | The threshold is set for each organization and means a silence that matters. A fixed reconnect delay would move it, and the rule would no longer match ADR-0044. |
| Read the stream state of the registry for the alert | ADR-0036 and ADR-0044 reject it: the registry of one instance gives another answer than the registry of its neighbour. |
| Store the start of the grace in the database | A row would turn a memory of one instance into a state of the installation. A second instance would then wait for a grace of a pass that it never ran. A restart would not start the grace again (FR-ID-08). |
| Keep one grace for the whole process | A pass that fails in one organization would hide the absent hosts of every organization for as long as it fails. |
| Judge the absence in the grace and open the alert later | The alert would open and close again in the grace, which sends the pair of messages that this record removes. |
