# Scenario 14.2.3: automatic recovery after a stale lock

- Started: 2026-10-15T17:35:55Z
- Revision: e86587a

| Step | Time (UTC) | Result | Evidence |
|---|---|---|---|
| 1. Start the bench with one agent | 17:36:24 | PASS | agent 01a140a2-ed58-73d9-9022-7fec22e7d721 |
| 2. Create the repository, the profile and the plan | 17:36:28 | PASS | repository 01a140a2-f678-7680-8ff3-256af8c430e6, plan 01a140a3-08b3-75dd-a0ea-3fadff2c1378 |
| 3. Run a backup of a 1 GB data set | 17:36:39 | PASS | job 01a140a3-30ca-7535-b580-e4eb85510df3 |
| 4. Stop the job with SIGKILL (target: agent) | 17:36:41 | PASS | SIGKILL to the agent process of the host (container 50364aa342c4d5e16e1ab86d8187d421e070b9306b2e3169bac54b7fabd533c2) at 17:36:41 |
| 5. The killed backup ends as failed | 17:42:45 | PASS | job 01a140a3-30ca-7535-b580-e4eb85510df3: failed failed |
| 6. The repository holds a stale lock | 17:42:46 | PASS | 1 lock(s) in the repository |
| 7. The control plane runs the automatic unlock (within 50 minutes) | 18:12:54 | PASS | unlock job 01a140c4-3b3f-718e-8267-020ce50b1766, waited 37 min since the start |
| 8. The stale lock is gone | 18:12:55 | PASS | 0 lock(s) in the repository |
| 9. The next backup succeeds | 18:12:58 | PASS | job 01a140c4-68e1-7319-925e-9a02f54d1827: succeeded succeeded |

Result: PASS
