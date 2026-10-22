---
title: Agent and control plane versions
description: Which agent versions work with which control plane version, and how to read the version marks.
---

The agent and the control plane speak the agent protocol. The protocol has a major and a minor number. The agent sends its protocol version, its package version and its restic version in the first message of each stream.

## The compatibility rule

- The agent and the control plane must have the same major number.
- The control plane accepts an agent with the minor number N and the minor number N-1. N is the minor number of the control plane.
- The control plane accepts an agent with a newer minor number. Optional features are on only when the agent reports the matching capability.
- The control plane refuses an agent with another major number or with a minor number older than N-1.

Upgrade the control plane first. Upgrade the agents after it. An agent of the previous minor version keeps working during the upgrade.

## What a refusal looks like

The control plane sends a typed refusal and closes the stream. The refusal names the reason, the protocol version of the control plane and the smallest minor number that it accepts. The agent logs the refusal with the text "the control plane does not accept the protocol version of this agent". Then it waits for 60 seconds before it connects again. Update the agent or the control plane so that both speak the same major version.

## Marks on the Hosts screen

The list of hosts shows the package version of each agent in the second line of the row, for example `agent 0.4.2`. If the agent package is older than the control plane, the line shows `agent 0.4.2, 0.5.0 is available`. A host with a version that the control plane refuses shows the status "Agent too old".

The page of the host shows the "Agent version" in the section "General" with a mark. The section "Connection and certificate" shows the "Protocol version" and the "Control plane version".

| Mark on the host page | Meaning |
|---|---|
| No mark | The agent version equals the control plane version, or it is newer. |
| Update available | The agent package is older than the control plane. The agent works. |
| Unsupported version | The control plane refuses the protocol version of the agent. The agent cannot connect. |

The agents API has the same facts: `agent_version`, `protocol_version`, `control_plane_version` and `version_status`.

## Build revision in the version

A release build has the package version of the workspace, for example `0.1.0`. The build adds the short revision after a plus sign, for example `0.1.0+3b21922`. A local build adds `+dev`. The command `restichub-agent --version` prints the version and the protocol version, for example `restichub-agent 0.1.0+3b21922 (protocol 1.1)`. The agent sends the same version when it enrolls and in the first message of each stream. The control plane exposes its own version in the same format.

A pre-release is older than its release, so `0.1.0-rc.4` is older than `0.1.0`. The revision does not change the comparison. The mark "Update available" depends on the package version only.

## Upgrade notes

### Hooks removed

The allow-list hooks no longer exist. The control plane drops the hook section of each profile when it upgrades. For each profile that had a hook, the audit log holds one record with the action `profile_hooks_removed`. The record names the profile, and the hook identifiers and phases. It holds no parameter value.

A plan that ran a hook now runs without it, and its jobs can still succeed. Check the audit log after the upgrade. Move each hook to a server script on a host with the trust mode. See [Server scripts](/restichub/guides/scripts/).

The agent no longer reads the file `restichub-agent-hooks.toml`. When the file still exists, the agent writes a warning with its path at each start. Delete the file when you no longer need it.

### Masked script values

A script variable can take the value of a profile parameter. The control plane sends a text value in the field `masked_value`, and the agent hides that value in the log of the script. An agent of an older package does not know the field, so it rejects the message. An agent that announces the capability `script-masked-value` knows the field.

The control plane reads the capabilities of the current connection. Thus an agent of an older package receives the value as a secret value, and it masks the value in the same way. The upgrade of the agent changes nothing for you.

### User name of a rest-server

Before this release, ResticHub sent the REST server password without a user name, so a rest-server with an `.htpasswd` file refused each job. For each REST server repository with a password, enter the user name of the account in the field "User name".
