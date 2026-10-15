# Scenario 14.2.3: automatic recovery after a stale lock

- Started: 2026-10-15T18:36:48Z
- Revision: 0ea6934

| Step | Time (UTC) | Result | Evidence |
|---|---|---|---|
| 1. Start the bench with one agent | 18:37:12 | PASS | agent 01a140da-a0fe-7387-94e4-244702531ca3 |
| 2. Create the repository, the profile and the plan | 18:37:14 | PASS | repository 01a140da-a28a-767a-bd0a-908c9241f4fa, plan 01a140da-aac2-73ab-a722-9570e69d510d |
| 3. Run a backup of a 1 GB data set | 18:37:18 | PASS | job 01a140da-b921-77ff-9dcd-3053fa4d90e7 |
| 4. Stop the job with SIGKILL (target: agent) | 18:39:52 | PASS | SIGKILL to the agent process of the host (container ea592149c7296c3b034e4ffec2bc3ff1b5d43232069f80666c0905aaeabc2787) at 18:39:52 |
| 5. The killed backup ends as failed | 18:39:54 | PASS | job 01a140da-b921-77ff-9dcd-3053fa4d90e7: failed failed |
| 6. The repository holds a stale lock | 18:39:55 | PASS | 1 lock(s) in the repository |
| 7. The control plane runs the automatic unlock (within 50 minutes) | 19:09:57 | PASS | unlock job 01a140f8-8bb1-75c9-9d4f-f88effa6dae7, waited 33 min since the start |
| 8. The stale lock is gone | 19:09:58 | PASS | 0 lock(s) in the repository |
| 9. The next backup succeeds | 19:10:00 | PASS | job 01a140f8-a08f-76df-ad56-f6429ccd5d11: succeeded succeeded |

Result: PASS
