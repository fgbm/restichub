# Scenario 14.2.1: a new tenant, from the creation to the first backup

- Started: 2026-10-15T16:49:25Z
- Revision: ede520d

| Step | Time (UTC) | Result | Evidence |
|---|---|---|---|
| 1. Start the control plane, PostgreSQL, the repository server and one agent host | 16:49:40 | PASS | agent-a=running cp=running postgres=running rest-server=running  |
| 2. Create the tenant and its first Admin (setup), sign in, create an enrollment token, enroll the agent | 16:49:47 | PASS | agent 01a14078-4495-757b-937d-5f4b99fc537a, 6b6b5b064586 restic 0.18.1 status active |
| 3. Create the repository | 16:49:47 | PASS | repository 01a14078-4ac2-771e-868e-6e3cf554a831 |
| 4. Initialize the repository | 16:49:51 | PASS | init job 01a14078-4afc-72b8-9a35-ef6215f9fa66: succeeded succeeded |
| 5. Create the profile for /data | 16:49:51 | PASS | profile 01a14078-5b69-7712-b3b0-ff0b0d9c1b1f |
| 6. Create the plan | 16:49:52 | PASS | plan 01a14078-5b97-77f2-9dc2-2e815dd997dd |
| 7. Run the plan: the first backup | 16:49:54 | PASS | backup job 01a14078-5bbd-7025-ab51-10ffdf39eefa: succeeded succeeded |
| 8. The control plane lists the snapshot (snapshots job on the agent) | 16:49:56 | PASS | 1 snapshot(s) |
| 9. restic on the host reads the same snapshot from the repository | 16:49:57 | PASS | 1 snapshot(s) read with restic 0.18.1 |
| 10. The job log is stored | 16:49:57 | WARN | 0 line(s) in the first page |
| 11. The audit log holds the actions of the scenario | 16:49:57 | PASS | 14 record(s) |

Result: PASS with findings (see the WARN rows)
