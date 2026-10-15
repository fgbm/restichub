# Scenario 14.2.4: agent revocation during a job

- Started: 2026-10-15T18:41:01Z
- Revision: 0ea6934

| Step | Time (UTC) | Result | Evidence |
|---|---|---|---|
| 1. Start the bench with one agent | 18:41:21 | PASS | agent 01a140de-6e35-709b-a02b-f69d811ee7be |
| 2. Create the repository, the profile and the plan | 18:41:25 | PASS | repository 01a140de-6f78-7357-aef1-3dfa3a0a4d77, plan 01a140de-7f9c-734d-9e64-efebce060c06 |
| 3. A backup runs on the agent | 18:41:31 | PASS | job 01a140de-94b2-7045-b09b-b7abef729074, restic pid 148, 1 stream connection(s) |
| 4. The control plane closes the stream in 5 s or less | 18:41:31 | PASS | stream closed 70 ms after the revoke call |
| 5. The cancellation stops the running backup | 18:41:32 | PASS | 0 restic process(es) left after at most 30 s; 0 snapshot(s) in the repository |
| 6. The job row of the revoked agent | 18:41:32 | WARN | status running  (a revoked agent cannot report; the row stays running until the lost-agent sweep) |
| 7. The next connection is refused (20 seconds of reconnect attempts) | 18:41:52 | PASS | 0 stream connection(s) open after 20 s |
| 8. The host shows as revoked | 18:41:52 | PASS | status revoked |
| 9. The audit log holds agent_revoke | 18:41:52 | PASS | 1 event(s) |

Result: PASS with findings (see the WARN rows)
