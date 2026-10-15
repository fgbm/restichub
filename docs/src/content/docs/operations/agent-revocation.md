---
title: Agent revocation
description: What happens when an Admin revokes an agent, and the limits of the revocation.
---

An Admin revokes an agent on the Hosts screen. The control plane uses the agent registry for the revocation. It uses no certificate revocation list (CRL) and no OCSP (ADR-0026).

## What a revoke does

- The control plane sets the status of the agent to `revoked` and writes the audit record `agent_revoke`.
- The control plane sends a cancellation for each job of the agent that is sent or running.
- The control plane closes the open stream of the agent. The close takes 5 seconds or less.
- The control plane refuses each next connection of the agent. The agent logs that the control plane refused its identity, and it retries at long intervals.
- A rotated certificate that the agent did not confirm is dropped.

## Several control plane instances

Each instance holds the streams of its own agents. The instance that serves the revoke closes its own streams at once. Each other instance checks the registry rows of its streams every 2 seconds, sends the cancellations and closes the stream of a revoked agent. The close takes 2 seconds plus the time of one database query.

## Limits

- The revocation of the server certificate of the control plane is out of scope for the MVP. The agents trust the installation root CA and the server certificate of the control plane. Replace a server certificate that leaked by a new one and restart the control plane.
- A revoke does not delete the host entry. The entry stays with its history.
