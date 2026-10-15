# Manual checklist: Windows hosts of bench S1 and scenario 14.2.2

Windows hosts cannot run in Docker Compose. A person runs this checklist on real or virtual machines. The result goes into the issue #43 (scenario 14.2.2) and into the issue #36 (the hosts of bench S1).

## Hosts

| Host | Needed for |
|---|---|
| Windows Server 2022 x64 | the file server of scenario 14.2.2, AC-AG-01 |
| Windows 11 x64 | AC-AG-01, a second agent |
| Debian 12 arm64 | AC-AG-01 (a Linux arm64 machine or a virtual machine) |
| Ubuntu 24.04 x64 | AC-AG-01 |

Each host reaches the control plane of the bench on its agent port (`CP_AGENT_PORT`, default 18443) and the repository. Publish the control plane on the network address of the bench host and set `RESTICHUB_AGENT_ENDPOINT` to that address before the first start.

## Scenario 14.2.2: a Windows file server with VSS and a restore on a different host

1. Install the agent on the Windows Server 2022 host: `restichub-agent.exe register --url <endpoint> --token <token> --ca-file <ca.pem>`. Record the output.
2. Check `restichub-agent.exe status`. Expected: the service runs and the connection is active.
3. Create a share with test files: at least 1000 files, some of them open for writing by another process during the backup.
4. Create a repository on the rest-server of the bench, a profile with the share path and VSS turned on, and a plan. Run the plan.
5. Expected: the job succeeds. The job log shows that restic created a VSS snapshot. Record the line.
6. Change two files and run the plan again. Record the snapshot list.
7. Install the agent on a second host (Windows 11 or Linux). Restore the latest snapshot to a directory inside a restore root of that host.
8. Compare the restored tree with the source: file count, sizes and SHA-256 of ten files. Record the result.
9. Add the report to the issue #43: each step, the time, the result and the evidence.

## AC-AG-01 on each host of bench S1

1. Install the agent. Expected: the service runs.
2. Restart the host. Expected: the service starts by itself and the host shows as connected.
3. Stop the agent process with a hard kill (Windows: `taskkill /F`; Linux: `kill -KILL <pid of the agent>`). Expected: the service starts again in 30 seconds or less.
4. Record the times in the issue.

## Evidence rules

- Record the host name, the operating system version, the agent version and the restic version.
- A step without evidence does not count.
