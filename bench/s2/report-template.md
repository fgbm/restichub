# Bench S2 acceptance report

Copy `out/report.md` into this layout and add the facts of the host. Put the result in the comment of issue #38.

## Run

| Item | Value |
|---|---|
| Date and time (UTC) | |
| Revision (`git rev-parse --short HEAD`) | |
| Host (OS, CPU, RAM) | |
| Docker version | |
| Control plane limits | `cpus: 2`, `mem_limit: 4g` |
| Agents | 500 |
| Job interval | 15 minutes, spread over the interval / burst |
| Run length | 4 hours (14400 s) |
| Command | |

## Result of AC-NFR-01

| Check | Target | Measured | Result |
|---|---|---|---|
| CPU p95 (percent of 2 vCPU) | below 70 % | | |
| RAM maximum | below 3 GB | | |
| Dispatch delay p95 | below 3 s | | |

Jobs finished: succeeded, failed. Fewest agents connected in the steady window: .

## Result of AC-ID-06 (restart)

| Item | Value |
|---|---|
| Agents connected before the restart | |
| Reconnect time (limit 300 s) | |
| Alerts of the kind `agent_offline` | |

## Notes

Say if the run broke a rule of design/acceptance-criteria.md, section 1.1 (for example a host that slept, or another heavy process on the host). Attach `resources.csv` and `metrics.csv`.
