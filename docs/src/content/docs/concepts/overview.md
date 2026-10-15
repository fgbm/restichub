---
title: Overview
description: Main parts and terms of ResticHub.
---

## Parts

| Part | Function |
|---|---|
| Control plane | Keeps the configuration, schedules jobs, sends jobs to agents, shows results. |
| Agent | Runs on each host. Receives signed jobs, runs restic, sends results. |
| Repository | A restic repository. The agent accesses it directly. |

## Terms

| Term | Meaning |
|---|---|
| Tenant | An isolated organization in one installation. |
| Profile | A backup definition: repository, paths, excludes, retention, scripts. |
| Plan | A profile assigned to agents, with a schedule. |
| Job | One run of a plan or a manual operation. |
| Step | One command in a job, for example `backup` or `forget`. |
| Script | A command text that runs before or after restic on a host with the trust mode. |
| Maintenance agent | The agent that runs `forget`, `prune` and `check` for a repository. |

## How a job runs

1. The control plane calculates the time of the next job from the plan schedule.
2. At that time the control plane puts the job in the queue.
3. When the repository is free, the control plane signs the job and sends it to the agent.
4. The agent verifies the signature.
5. The agent runs the steps in sequence and sends progress to the control plane.
6. The agent sends the result. The control plane shows it on the dashboard.
