# ADR-0004: Hooks only from an agent-side allow-list

- Status: superseded by [ADR-0039](0039-remove-allow-list-hooks.md). [ADR-0037](0037-server-scripts-on-trusted-hosts.md) replaced it for hosts with the local trust mode.
- Date: 2026-10-12
- Requirements: FR-HK-01 to FR-HK-07, NFR-SEC-03

## Context

Hooks run commands on hosts before and after a backup. If the control plane can send any command, a compromised control plane can execute code on all hosts. Earlier incidents in similar systems show this risk: SaltStack CVE-2020-11651 and Velociraptor CVE-2026-5329.

## Decision

The agent runs only hooks from a local allow-list file. Only root or Administrators can write this file. The control plane, self-update and jobs cannot change it.

Each hook has an ID, an executable, an argument template and a parameter schema. The control plane refers to a hook by ID and sends parameters. The agent validates the parameters and starts the hook without a shell.

## Consequences

- A compromised control plane cannot execute arbitrary commands on hosts.
- A host administrator must install the allow-list on each host. Packages and configuration management tools can do this.
- The allow-list is the only local configuration that the agent keeps on purpose. One more root-only file follows the same rule: the list of restore roots ([ADR-0035](0035-restore-and-snapshot-browsing.md)).
- The UI shows only hooks that all target agents have.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Arbitrary commands, signed and approved | A compromised signing process still gives code execution on all hosts. |
| No hooks | Application consistency (database dumps, service stop) is necessary. |
