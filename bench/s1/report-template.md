# Acceptance report: <AC-SC-04 | AC-NFR-06 | AC-AG-02>

Copy this file into the issue. Fill each field from `report.md` of the run directory.

## Run

| Field | Value |
|---|---|
| Criterion | |
| Issue | |
| Revision (git) | |
| Start (UTC) | |
| End (UTC) | |
| Duration | |
| Host of the bench | CPU cores, RAM, operating system |
| Command | `bench/s1/run.sh <profile> <out>` |
| Agents | number, restic versions |
| Repository | rest-server 0.14.0, one repository |
| Plan | backup each 15 minutes; `forget` and `prune` each 2 hours; `check` each day |

## Result

| Measure | Expected | Measured |
|---|---|---|
| Jobs with status `failed` because of a lock (`repository_locked`, `dead_letter`) | 0 | |
| `check` jobs that did not succeed | 0 | |
| Backup jobs that failed (all causes) | explain each | |
| Lowest number of online agents in the samples | all | |
| False alerts | 0 | |
| Connections other than the configured ones (AC-NFR-06) | 0 | |
| `restic` processes on an idle host (AC-AG-02) | 0 | |

## Notes

- Each failed job with its cause:
- Each alert and the decision (true or false):
- Each interruption of the run:
- Peak load of the control plane (CPU, memory):

## Verdict

PASS or FAIL, with the reason. Attach `report.md`, `result.json`, `samples.csv`, `failed-jobs.tsv` and, for AC-NFR-06, `capture-report.md`.
