# Bench S1 smoke (20 agents, 45 min)

- Run directory: `out/smoke-manual`
- Report made: 2026-10-15T17:27:25Z
- Setup finished: 2026-10-15T16:40:43Z
- Repository: rest-server 0.14.0, one repository
- Agents: 20 registered, 20 online at the end, 20 the lowest online count in the samples
- Restic versions on the agents: 0.18.1 x10, 0.19.1 x10

## Jobs

```json
{
  "jobs_total": 319,
  "by_kind_status": {
    "backup:succeeded": 313,
    "check:succeeded": 2,
    "forget:succeeded": 3,
    "init:succeeded": 1
  },
  "failed_by_code": {},
  "lock_failures": 0,
  "check_jobs": 2,
  "check_failures": 0,
  "backup_jobs": 313,
  "backup_failures": 0,
  "backup_seconds_p50": 2,
  "backup_seconds_p95": 4,
  "queue_wait_seconds_p95": 4,
  "first_job_at": "2026-10-15T16:40:39.223915Z",
  "last_job_at": "2026-10-15T17:27:00.633996Z"
}
```

## AC-SC-04 verdict: PASS

- Jobs failed because of a lock (`repository_locked` or `dead_letter`): 0
- `check` jobs that did not succeed: 0
- Alerts in the list (a person marks the false ones): 0
- Highest restic process count on the agents (idle run: expected 0): n/a
- Control plane peak: 5.73% CPU, 58.05 MiB

## Failed, expired and dead-letter jobs

```
id	kind	status	result_code	agent	message
```

## Warnings of the control plane

```
```
