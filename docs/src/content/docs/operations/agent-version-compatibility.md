---
title: Agent and control plane versions
description: Which agent versions work with which control plane version, and how to read the version marks.
---

The agent and the control plane speak the agent protocol (ADR-0008). The protocol has a major and a minor number. The agent sends its protocol version, its package version and its restic version in the first message of each stream (FR-PR-09).

## The compatibility rule

- The agent and the control plane must have the same major number.
- The control plane accepts an agent with the minor number N and the minor number N-1. N is the minor number of the control plane.
- The control plane accepts an agent with a newer minor number. Optional features are on only when the agent reports the matching capability.
- The control plane refuses an agent with another major number or with a minor number older than N-1.

Upgrade the control plane first. Upgrade the agents after it. An agent of the previous minor version keeps working during the upgrade.

## What a refusal looks like

The control plane sends a typed refusal and closes the stream. The refusal names the reason, the protocol version of the control plane and the smallest minor number that it accepts. The agent logs the refusal with the text "the control plane does not accept the protocol version of this agent". Then it waits for 60 seconds before it connects again. Update the agent or the control plane so that both speak the same major version.

## Marks on the Hosts screen

The Hosts screen shows the package version of each agent and a mark:

| Mark | Meaning |
|---|---|
| No mark | The agent version equals the control plane version, or it is newer. |
| Update available | The agent package is older than the control plane. The agent works. |
| Unsupported version | The control plane refuses the protocol version of the agent. The agent cannot connect. |

The agents API has the same facts: `agent_version`, `protocol_version`, `control_plane_version` and `version_status`.

## Build revision in the version

A release build has the package version of the workspace, for example `0.1.0`. The build adds the short revision after a plus sign, for example `0.1.0+3b21922`. A local build adds `+dev`. The command `restichub-agent --version` prints the version. The agent sends the same version when it enrolls and in the first message of each stream. The control plane exposes its own version in the same format.

The revision does not change the comparison. The mark "Update available" depends on the package version only.
