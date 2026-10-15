# ADR-0040: Remove the allow-list hooks

- Status: accepted
- Date: 2026-10-16
- Requirements: FR-HK-01, FR-HK-05, FR-HK-08 to FR-HK-12, NFR-SEC-03
- Supersedes: [ADR-0004](0004-hook-allow-list.md), and decision 6 of [ADR-0037](0037-server-scripts-on-trusted-hosts.md)

## Context

[ADR-0037](0037-server-scripts-on-trusted-hosts.md) lets a user write any script in the web UI. A script runs before or after a backup, with its own log. It runs only on a host with the local trust mode. This covers every use of a hook: a database dump, a service stop and start, a notification. The allow-list of [ADR-0004](0004-hook-allow-list.md) stayed as the mode for hosts without the trust mode. It needs a second kind of step, a second file on each host, a parameter schema and a second editor in the UI. The owner decided that two ways to run a command before or after a backup are one too many.

## Decision

1. The allow-list hooks do not exist. There is no allow-list file, no `hook` step, no `hooks-v1` capability and no hook section in a profile.
2. A host without the trust mode runs no command from the control plane. It still runs the typed restic steps.
3. The trust mode, the `script` step and its rules stay as [ADR-0037](0037-server-scripts-on-trusted-hosts.md) defines them.
4. The ownership rule of the allow-list file moves to the trust mode file (FR-HK-01). Only root or Administrators write it.

## Consequences

- Requirements FR-HK-02, FR-HK-03, FR-HK-04, FR-HK-06 and FR-HK-07 are removed. Criteria AC-HK-01, AC-HK-03, AC-HK-04, AC-HK-05 and AC-HK-06 are removed. AC-HK-02 now checks the trust mode file.
- The documentation page about allow-list hooks is removed. The terms "hook" and "allow-list" leave the glossary.
- The error code `INVALID_HOOK_PARAMETERS` and the `hook` step leave the protocol.
- Code follow-up, not part of this change. Remove the allow-list code of the agent. Remove the hook step from the compiler and the protocol. Remove the hook section of the profile in the web UI. Remove the tests of the removed criteria.
- A host that must not run server scripts has the trust mode off. It cannot run a command before or after a backup at all. Users with such a host take the application consistency from outside ResticHub.
- Older ADRs and notes that say "hook" mean a script on a trusted host.

## Rejected options

| Option | Reason |
|---|---|
| Keep the allow-list for hosts without the trust mode | Two mechanisms for one task. The allow-list needs an installation on each host and a second editor. |
| Allow scripts on every host | A compromised control plane would run code on every host (ADR-0037). The trust mode stays the explicit consent of the host administrator. |
