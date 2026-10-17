---
title: Status and reason codes
description: The step statuses, the reason codes of failed jobs and their texts in the web interface.
---

This page lists the codes that the control plane stores for a job and its steps. For the job states, see [Job lifecycle](/restichub/concepts/job-lifecycle/#job-states).

## Step statuses

| Code | Text in the web interface |
|---|---|
| `cancelled` | Cancelled |
| `config_error` | Configuration error |
| `failed` | Failed |
| `retry` | Repository locked |
| `skipped` | Skipped |
| `success` | Succeeded |
| `warning` | Warning |

## Refused jobs

If the control plane cannot build a job, the job fails with a reason code:

| Reason | Meaning |
|---|---|
| `compile_failed` | The profile no longer compiles for the agent. |
| `agent_facts_invalid` | The facts of the agent are not usable. |
| `agent_capability_missing` | The agent is too old for the step. It does not know a field that the step needs. |
| `forget_without_tags` | The retention step has no tag filter. The control plane does not send it. |
| `step_missing` | The profile no longer holds the step of the job. |
| `agent_revoked` | The agent is revoked. |
| `plan_missing`, `plan_changed` | The plan is deleted, or it moved to another agent. |
| `profile_missing`, `repository_missing`, `repository_changed` | An object of the job is deleted or changed. |
| `secret_unavailable` | A secret of the repository cannot be read. |
| `signing_failed` | The control plane could not sign the job. |

## Reasons and result codes

A failed or refused job shows a reason under its title on the page of the job. These are the texts:

| Code | Text in the web interface |
|---|---|
| `agent_capability_missing` | The agent is too old for this job. Update the agent. |
| `agent_facts_invalid` | The facts of the host are not usable. |
| `agent_lost` | The host went offline during the job. |
| `agent_revoked` | The host is revoked. |
| `cancelled` | The job was cancelled. |
| `compile_failed` | The profile no longer fits the host. |
| `expired` | The job waited too long and expired. |
| `forget_selected_nothing` | Forget selected no snapshots |
| `forget_without_tags` | The retention has no tag filter. The job was not sent. |
| `hook_failed` | A script or a hook ended with an error. |
| `interrupted` | Interrupted: the agent restarted while scripts ran |
| `job_invalid` | The job did not pass the validation. |
| `listing_missing` | The host sent no result for this request. |
| `lock_retries_exhausted` | The repository stayed locked after every retry. |
| `lock_retry` | The repository was locked. A new attempt is queued. |
| `max_duration_exceeded` | The job was stopped: it ran longer than its maximum duration. |
| `not_acknowledged` | The host never confirmed the job. |
| `plan_changed` | The plan moved to another host. |
| `plan_missing` | The plan is deleted. |
| `profile_missing` | The profile is deleted. |
| `repository_changed` | The profile now uses another repository. |
| `repository_locked` | The repository is locked by another process. |
| `repository_missing` | The repository is deleted. |
| `request_missing` | The job lost its request. |
| `restore_target_denied` | The host refused the target path. It lies outside the restore roots or holds a file of the agent. |
| `secret_unavailable` | A repository secret is not available. |
| `signed_job_unavailable` | The stored job is not readable. |
| `signing_failed` | The control plane could not sign the job. |
| `stalled` | The job was stopped: restic made no progress for too long. |
| `step_failed` | restic stopped with an error. |
| `step_missing` | The profile no longer holds this step. |
| `step_not_started` | restic did not start. The job log has the cause. |
| `succeeded_with_warnings` | Finished with warnings. |
| `timed_out` | The job ran longer than its maximum duration. |
| `unacknowledged_expired` | The host did not confirm the job before it expired. |
| `unsupported_kind` | The queue does not run this kind of job. |
| `wrong_password` | The repository password or the access keys are wrong. |

An alert about a failed job uses a short text of the code:

| Code | Text in the alert |
|---|---|
| `agent_lost` | the host went offline |
| `cancelled` | cancelled |
| `expired` | the job expired |
| `hook_failed` | script or hook error |
| `interrupted` | interrupted by an agent restart |
| `max_duration_exceeded` | maximum duration exceeded |
| `repository_locked` | repository locked |
| `repository_missing` | repository not found |
| `stalled` | no progress |
| `step_failed` | restic error |
| `step_not_started` | restic did not start |
| `wrong_password` | wrong password or keys |
