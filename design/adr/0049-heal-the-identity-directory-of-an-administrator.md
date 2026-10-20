# ADR-0049: Heal the administrator entry on the identity directory

- Status: accepted
- Date: 2026-10-20
- Requirements: FR-AG-01, FR-AG-04
- Supersedes: decision 3 of [ADR-0048](0048-administrator-access-entries-on-windows.md) (the identity directory and its backup are not healed)

## Context

ADR-0048 lets the service remove the File Explorer entry of a local administrator from the agent directories at its start. Decision 3 kept the identity directory `%ProgramData%\ResticHub\Agent` and its backup `Agent.old` out of the heal. An entry there stopped the service with the exit code 78, and only `register --force` with a new token repaired the host.

An administrator who opens `%ProgramData%\ResticHub` usually opens `Agent` next. File Explorer asks again and adds a second entry on `Agent`. Thus the most frequent case did not heal, and each such click needed a new registration.

An entry can also come while the service runs. Then the next break of the stream checked the identity and stopped the service with the exit code 78. The service manager does not restart a service after this code. The heal at the next start did not run until a restart of the computer. A stop of the service also checked the identity first, so a normal stop reported the refusal.

## Decision

1. The service heals the identity directory and its backup as the other data directories of ADR-0048. It removes an explicit allow entry when its trustee is a user account that the computer confirms as a member of the local Administrators group. It also removes the copies of that entry on the children.
2. The order stays "check first, remove second". Before the removal the service runs the checks and the repair that `install-service` runs for a level that another account could write. These checks ignore the entry and its copies. They refuse a name that the agent does not know, a link and an entry of a foreign owner. They also refuse an identity file that another account could change. A refusal keeps the entry, and the service stops with the exit code 78. Each later start then refuses again.
3. After the removal the service checks the identity again. It writes a warning to the `ResticHub` event log and to the agent log. The warning names the path and the account, and it tells that the identity was exposed to that account. It also tells how to register the host again if the account is not trusted.
4. Each other entry on the identity directory or its backup keeps the refusal with the exit code 78 and `register --force`. This covers a user account outside Administrators, a group, a trustee that the computer cannot verify and an entry that the agent does not parse.
5. The stream loop of a running service does a stop before it checks the identity, so a stop of the service is a clean stop. On Windows, the running service can find an identity that another account could change. Then the process stops with an error, and the recovery actions of the service restart it. The new start runs the heal. A refusal there stops the service with the exit code 78, so no restart loop occurs.
6. `install-service` and `register` without `--force` run the same heal of the identity directory and its backup before their repair. The checks, the order and the warning are the same as at the start of the service. A refusal keeps the entry, and the command stops with the message of the check.

## Consequences

- An administrator who opens `%ProgramData%\ResticHub\Agent` in File Explorer gets a working service at the next start, without a new token.
- The account could read and change the identity between the click and the next start. The heal does not undo this. The warning tells the administrator, who decides if a new registration is necessary. The checks before the removal refuse an identity file that the account replaced, because the account owns the new file. They also refuse a name that the account added.
- An entry that File Explorer adds while the service runs heals at the next break of the stream, after a restart from the service manager. Until that break the account keeps its access.
- `install-service` keeps its rule for each other entry. It leaves an identity directory that another account could change as found. Only a new registration replaces it. The command also heals an entry of an administrator that File Explorer added while the service ran. Thus an upgrade with `install-service` needs no new token.
- The heal is for Windows only. A running Linux agent still stops with the exit code 78 when another account could change the identity. Decision 5 applies to Linux too, so a stop of the service there is a clean stop with the exit code 0.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Keep decision 3 of ADR-0048 | The most frequent Explorer click needs a new token and a new registration. |
| Heal the entry while the service runs | The service holds no timer for a repair. The heal runs at the start only, with the checks of `install-service`. |
| Keep the exit code 78 for an exposure that the running service finds | The service stays stopped, and the heal at the next start does not run until a restart of the computer. |
| Heal the identity entry of every account | A non-elevated process of an account outside Administrators could have changed the identity. Only a new registration replaces it. |
