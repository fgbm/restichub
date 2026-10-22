---
title: Monitor the control plane
description: Which health endpoint to use for a container, a load balancer and an external monitor, and what a failing background task means.
---

The control plane has two health endpoints. Each endpoint answers a different question.

| Endpoint | Question | Answer |
|---|---|---|
| `/healthz` | Does the process run? | `200` while the process answers. |
| `/readyz` | Does the control plane work? | `200` when the database answers and no background task is failing. Otherwise `503`. |

## Background tasks

The control plane runs background tasks. They dispatch jobs, run the schedules, send alerts, remove old data and flush the host status. A supervisor watches each task.

If a task panics or stops, the supervisor writes an `ERROR` line with the task name to the log. The supervisor starts the task again after a pause. The pause starts at 1 second and doubles up to 60 seconds.

While a task is failing, `/readyz` answers `503`. A restarted task counts as healthy after it has run for 60 seconds without a stop. A task that runs once an hour runs its pass at once after a restart, so a pass that always breaks keeps `/readyz` at `503`.

## Choose the endpoint

- Use `/readyz` in an external monitor. The control plane does not alert about itself. A monitor that polls `/readyz` once a minute shows a failing task.
- Use `/healthz` for the health check of a container and for the probe of a load balancer.

:::caution
Do not use `/readyz` as a readiness probe or a load balancer check when you run one instance. A `503` from `/readyz` removes the only instance from the service. The web interface, the API and the agent port then stop, although only one background task failed.
:::

The reference stack checks `/healthz`. The container stays healthy while a task is failing, and the monitor reports the failure.

## Find the failing task

1. Read the log of the control plane:

   ```sh
   docker compose logs cp | grep "a background task stopped"
   ```

2. Find the field `task` in the line. It names the failing task.
3. Read the field `reason` in the same line. It holds the panic text or the error.

If `/readyz` stays at `503` and the line repeats, report the line to the project.

## Related

- [Install the control plane](/restichub/getting-started/install-control-plane/)
- [Alerts and notification channels](/restichub/guides/alerts/)
- [Security for operators](/restichub/operations/security-for-operators/)
