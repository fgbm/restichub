# ADR-0048: Self-repair of administrator access entries on Windows

- Status: accepted
- Date: 2026-10-20
- Requirements: FR-AG-01, FR-AG-04, FR-IN-07, FR-IN-08
- Related: [ADR-0038](0038-agent-downloads-restic.md) (the restic record), [ADR-0037](0037-server-scripts-on-trusted-hosts.md) (the trust mode file), [ADR-0045](0045-repository-roots-for-local-repositories.md) (the roots files)

## Context

The agent keeps `%ProgramFiles%\ResticHub` and `%ProgramData%\ResticHub` with one protected access control list: full control for SYSTEM and Administrators, and no entry for any other account. A non-elevated token of an administrator holds the Administrators group as deny-only. Thus the administrator has no list right on these directories, and File Explorer asks for consent when the administrator opens one.

File Explorer then adds an explicit full-control entry for the account of the administrator. Windows propagates a copy to the children that are not protected. The protected levels and files keep their list, so the identity, restic and the record stay closed. The service checks its directories at each start. It then stops with the exit code 78, often weeks after the click, and after a reboot. Until then the account could change the directory without elevation, and a library that it placed in `Agent` ran as SYSTEM at that start.

The existing repair replaces the whole program tree and downloads restic again. It needs network access or a restic archive. It is a large action for an entry that Windows adds on purpose.

## Decision

1. The program tree grants the read and execute access to Users: `%ProgramFiles%\ResticHub`, `Agent` and each program file that an install writes. Only SYSTEM and Administrators write there. The read right is no change right, so each existing check keeps its result. `install-service` and `register` add this entry to a level of an existing install that passed the check.
2. At its start the service removes an allow entry from an agent directory when the entry is explicit. Its trustee must be a user account in the local Administrators group, and a nested group counts. The Authorization API builds the group list of the account. The service removes the entry from these directories: `%ProgramFiles%\ResticHub`, `Agent`, `%ProgramData%\ResticHub`, `Agent`, `Agent.old`, `Agent-logs`, `Agent\cache` and the log spool. It then writes a warning to the `ResticHub` event log and to the agent log. The warning names the path and the account. The service continues.
3. After it removed an entry of a program level, the service checks the program tree. It refuses a name that the agent does not know, a link and an open entry. It then checks the identity again and verifies restic against its record. A failure stops the service with the exit code 78, as before.
4. Every other entry keeps the refusal with the exit code 78. This covers a user account outside Administrators, a group, an inherited entry and an entry that the agent does not parse. It also covers a trustee that the computer cannot verify and every file entry. The two roots files stay a refusal, because a non-elevated process of the account could have changed their content.
5. The command `install-service` repairs a directory that a non-administrator opened, as before. `register --force` replaces an identity that another account could change.
6. `%ProgramData%\ResticHub` keeps no read entry for Users, so the names of the files there do not leak. The log stays readable by SYSTEM and Administrators only.

## Consequences

- An administrator who opens an agent folder in File Explorer and clicks "Continue" gets a working service at the next start. The entry does not wait for an install, a download and a restart of the computer.
- The account had write access to the directory between the click and the next start. A library that it placed in `Agent` ran as SYSTEM at that start. The refusal of the old behaviour did not prevent this either: it stopped the service after the library loaded. The heal stops a later use of the entry, and the checks name a planted file.
- A handle that a process opened on the directory before the heal keeps its access until it closes. This is the same residual as the repair in place.
- A domain account in Administrators gets the heal only while the domain controller answers, and while the computer account can read the group list. A domain controller that is not reachable, a hardened domain and an Entra ID account give the refusal with the exit code 78. Then `install-service` repairs the tree, or the administrator removes the entry with `icacls`.
- A disabled account that stays in Administrators counts as an administrator. The entry gives a disabled account no access.
- The program tree is readable by every local user. It holds public binaries only. The restic record, the identity and the log stay closed.
- The heal runs only for the service. A console run of `run` without elevation cannot change the access control, so it changes nothing and the checks refuse as before.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Give Users the read access on `%ProgramData%\ResticHub` | The names of the files there show whether the trust mode is on. |
| Give Users the read access on `Agent-logs` | A read of the log shows paths and host names to every local user, and FR-AG-04 says SYSTEM and Administrators only. |
| Remove each entry that another account holds | A non-elevated process of that account could have changed the directory or a file already. The administrator must look at it first. |
| Heal the entries of files too | A file that another account could change has content that the agent must not trust. Only a new registration or an install replaces it. |
| Repair the directory at the click, from File Explorer | The agent does not run there, and the consent dialog belongs to Windows. |
| Check the directories periodically while the service runs | The service holds no timer for a repair. The next start is enough: the checks refuse a later change. |
| Let `install-service` replace the tree, as today | It downloads restic again and needs network access for an entry that Windows adds on purpose. |
